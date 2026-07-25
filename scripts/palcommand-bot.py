"""
palcommand-bot.py - Discord bot for the PAL COMMAND Palworld server.

Slash commands:
    /status    live server state (players, day, FPS, uptime)
    /players   who is online right now
    /mods      the guild mod set + how to install it
    /backup    when the last backup ran and whether it is off-site

Background monitors (post to the alert channel on change):
    * server up/down  - polled every 2 minutes, alerts only on state change
    * mod releases    - announces additions, updates, disables, and removals
    * config changes  - changed key names only; values and secrets are never posted

Secrets are never stored here. Start-PalBot.ps1 decrypts them from DPAPI and
passes them in as environment variables.
"""

import asyncio
import json
import logging
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timezone

import aiohttp
import discord
from discord import app_commands
from discord.ext import tasks

log = logging.getLogger("palbot")
logging.basicConfig(
    level=logging.INFO,
    format="[%(asctime)s] %(levelname)s %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)

TOKEN = os.environ.get("PALCMD_DISCORD_TOKEN", "").strip()
CHANNEL_ID = int(os.environ.get("PALCMD_DISCORD_CHANNEL", "0") or 0)
ADMIN_PW = os.environ.get("PALCMD_ADMIN_PW", "")
MODS_MANIFEST = os.environ.get(
    "PALCMD_MODS_MANIFEST",
    "https://raw.githubusercontent.com/luibots/palworld-mods/master/mods.json",
)
MODS_REPOSITORY = "https://github.com/luibots/palworld-mods"
# Local mods repo on the host - used to build the self-contained bundle for /getmods,
# so distribution stays private (nothing served from a public repo).
MODS_REPO = os.environ.get(
    "PALCMD_MODS_REPO",
    r"C:\Users\llllllllllllllllllll\projects\palworld-mods",
)

AMBER = 0xF5A524
GREEN = 0x22C55E
RED = 0xEF4444
GREY = 0x6B7280

CFG_DIR = pathlib.Path(os.environ["APPDATA"]) / "com.luibots.palcommand"
SETTINGS = CFG_DIR / "settings.json"
AUTO_DIR = CFG_DIR / "auto"
CONFIG_EVENT_QUEUE = AUTO_DIR / "discord-events.jsonl"
LAST_CONFIG_CHANGE = AUTO_DIR / "last-config-change.json"


def load_settings() -> dict:
    try:
        # settings.json may carry a BOM (written by PowerShell)
        return json.loads(SETTINGS.read_text(encoding="utf-8-sig"))
    except Exception as e:  # noqa: BLE001
        log.error("could not read settings.json: %s", e)
        return {}


def load_last_config_change() -> dict | None:
    try:
        return json.loads(LAST_CONFIG_CHANGE.read_text(encoding="utf-8"))
    except Exception:  # noqa: BLE001
        return None


def claim_config_events() -> tuple[list[dict], list[pathlib.Path]]:
    AUTO_DIR.mkdir(parents=True, exist_ok=True)
    claimed = list(AUTO_DIR.glob("discord-events.*.processing"))
    if CONFIG_EVENT_QUEUE.exists():
        processing = AUTO_DIR / f"discord-events.{os.getpid()}.{uuid.uuid4().hex}.processing"
        try:
            os.replace(CONFIG_EVENT_QUEUE, processing)
            claimed.append(processing)
        except OSError:
            pass

    events = []
    for path in claimed:
        try:
            for line in path.read_text(encoding="utf-8").splitlines():
                try:
                    event = json.loads(line)
                    if event.get("event_type") == "config_changed":
                        events.append(event)
                except json.JSONDecodeError:
                    log.warning("ignored malformed Discord event in %s", path.name)
        except OSError as e:
            log.warning("could not read Discord event queue %s: %s", path.name, e)
    return events, claimed


try:
    from zoneinfo import ZoneInfo

    PACIFIC = ZoneInfo("America/Los_Angeles")
except Exception:  # noqa: BLE001 - tzdata missing; fall back to machine local time
    PACIFIC = None


def fmt_pacific(dt: datetime) -> str:
    """Format a datetime as friendly 12-hour Pacific time, e.g. 'Jul 22, 6:45 PM PT'."""
    if PACIFIC is not None:
        if dt.tzinfo is None:
            dt = dt.astimezone()  # attach machine local tz, then convert
        dt = dt.astimezone(PACIFIC)
        label = dt.tzname() or "PT"  # PST or PDT depending on the date
    else:
        label = "local"
    return dt.strftime("%b %d, %I:%M %p").replace(" 0", " ") + f" {label}"


def world_to_map(x, y):
    """Palworld world units -> the coordinates shown on the in-game map.

    Calibrated 2026-07-22 against a player standing at a known in-game position
    (reported 252, -502; this returns 250.5, -501.2 - the ~1 unit residual is the
    player moving between the report and the reading).

    NOTE: the offsets are easy to transpose. X uses 158000, Y uses 123000.
    """
    try:
        return (float(y) - 158000.0) / 460.0, (float(x) + 123000.0) / 460.0
    except (TypeError, ValueError):
        return None, None


def human_uptime(seconds) -> str:
    try:
        s = int(seconds)
    except (TypeError, ValueError):
        return "unknown"
    if s <= 0:
        return "unknown"
    d, rem = divmod(s, 86400)
    h, rem = divmod(rem, 3600)
    m = rem // 60
    if d:
        return f"{d}d {h}h"
    if h:
        return f"{h}h {m}m"
    return f"{m}m"


async def fetch_manifest():
    """Read the mod manifest - local repo first (works while the repo is private),
    falling back to the public URL if a local copy isn't present."""
    local = os.path.join(MODS_REPO, "mods.json")
    if os.path.isfile(local):
        try:
            with open(local, "r", encoding="utf-8-sig") as f:
                return json.load(f)
        except Exception as e:  # noqa: BLE001
            log.debug("local manifest read failed: %s", e)
    try:
        timeout = aiohttp.ClientTimeout(total=15)
        async with aiohttp.ClientSession(timeout=timeout) as s:
            async with s.get(MODS_MANIFEST) as r:
                if r.status != 200:
                    return None
                return await r.json(content_type=None)
    except Exception as e:  # noqa: BLE001
        log.debug("manifest fetch failed: %s", e)
        return None


class PalAPI:
    """Thin async client for the Palworld REST API."""

    def __init__(self, base: str, admin_pw: str):
        self.base = (base or "").rstrip("/")
        self.auth = aiohttp.BasicAuth("admin", admin_pw) if admin_pw else None

    async def _get(self, path: str):
        if not self.base:
            return None
        url = f"{self.base}/v1/api/{path}"
        try:
            timeout = aiohttp.ClientTimeout(total=8)
            async with aiohttp.ClientSession(timeout=timeout) as s:
                async with s.get(url, auth=self.auth) as r:
                    if r.status != 200:
                        log.warning("%s -> HTTP %s", path, r.status)
                        return None
                    return await r.json(content_type=None)
        except Exception as e:  # noqa: BLE001
            log.debug("%s failed: %s", path, e)
            return None

    async def info(self):
        return await self._get("info")

    async def metrics(self):
        return await self._get("metrics")

    async def players(self):
        data = await self._get("players")
        if data is None:
            return None
        if isinstance(data, dict):
            return data.get("players", [])
        return data

    async def _post(self, path: str, payload: dict) -> bool:
        if not self.base:
            return False
        url = f"{self.base}/v1/api/{path}"
        try:
            timeout = aiohttp.ClientTimeout(total=10)
            async with aiohttp.ClientSession(timeout=timeout) as s:
                async with s.post(url, auth=self.auth, json=payload) as r:
                    return r.status in (200, 204)
        except Exception as e:  # noqa: BLE001
            log.debug("POST %s failed: %s", path, e)
            return False

    async def announce(self, message: str) -> bool:
        return await self._post("announce", {"message": message})

    async def shutdown(self, waittime: int, message: str) -> bool:
        return await self._post("shutdown", {"waittime": waittime, "message": message})


class PalBot(discord.Client):
    def __init__(self):
        intents = discord.Intents.default()
        super().__init__(intents=intents)
        self.tree = app_commands.CommandTree(self)
        self.settings = load_settings()
        self.api = PalAPI(self.settings.get("rest_url", ""), ADMIN_PW)
        self.last_up = None          # None = unknown yet
        self.known_mods = None       # None = not primed yet

    async def setup_hook(self):
        # NOTE: deliberately no global sync. Registering both globally and per-guild makes
        # every command appear TWICE in the Discord picker. Guild-only sync (in on_ready)
        # is also instant, whereas global commands take up to an hour to propagate.
        self.watch_server.start()
        self.watch_mods.start()
        self.watch_config_events.start()

    async def alert(self, embed: discord.Embed, text: str = ""):
        if not CHANNEL_ID:
            return False
        ch = self.get_channel(CHANNEL_ID)
        if ch is None:
            try:
                ch = await self.fetch_channel(CHANNEL_ID)
            except Exception as e:  # noqa: BLE001
                log.error("alert channel %s unreachable: %s", CHANNEL_ID, e)
                return False
        # Always send a plain-text mirror: clients with embeds turned off see nothing otherwise.
        if not text:
            text = f"**{embed.title}**" + (f" - {embed.description}" if embed.description else "")
        try:
            await ch.send(content=text[:1900], embed=embed)
            return True
        except Exception as e:  # noqa: BLE001
            log.error("could not post alert: %s", e)
            return False

    # ---------------------------------------------------------- monitors

    @tasks.loop(minutes=2)
    async def watch_server(self):
        info = await self.api.info()
        up = info is not None
        if self.last_up is None:
            self.last_up = up
            log.info("initial server state: %s", "UP" if up else "DOWN")
            return
        if up == self.last_up:
            return
        self.last_up = up
        if up:
            e = discord.Embed(
                title="Server is back UP",
                description=f"**{info.get('servername', 'Palworld')}** is responding again.",
                colour=GREEN,
                timestamp=datetime.now(timezone.utc),
            )
        else:
            e = discord.Embed(
                title="Server is DOWN",
                description="The Palworld server stopped responding. It may be restarting, or it may need a kick from the Host Havoc panel.",
                colour=RED,
                timestamp=datetime.now(timezone.utc),
            )
        await self.alert(e)

    @watch_server.before_loop
    async def _before_watch_server(self):
        await self.wait_until_ready()

    @tasks.loop(seconds=10)
    async def watch_config_events(self):
        events, claimed = claim_config_events()
        retry = []
        try:
            for event in events:
                keys = sorted({str(key) for key in event.get("changed_keys", []) if key})
                if not keys:
                    continue
                source = str(event.get("source") or "PAL COMMAND")
                key_text = ", ".join(f"`{key}`" for key in keys)
                e = discord.Embed(
                    title="Server config changed",
                    description=f"{source} changed {len(keys)} setting(s).",
                    colour=AMBER,
                    timestamp=datetime.now(timezone.utc),
                )
                e.add_field(name="Changed settings", value=key_text[:1024], inline=False)
                e.add_field(
                    name="Next step",
                    value="Restart through PAL COMMAND Safe Restart when the server is empty.",
                    inline=False,
                )
                text = (
                    f"**Server config changed** - {source} changed: "
                    f"{', '.join(keys)}. Values are intentionally hidden."
                )
                if await self.alert(e, text):
                    LAST_CONFIG_CHANGE.write_text(
                        json.dumps(event, indent=2),
                        encoding="utf-8",
                    )
                else:
                    retry.append(event)
        finally:
            for path in claimed:
                try:
                    path.unlink(missing_ok=True)
                except OSError:
                    pass
            if retry:
                with CONFIG_EVENT_QUEUE.open("a", encoding="utf-8") as queue:
                    for event in retry:
                        queue.write(json.dumps(event, separators=(",", ":")) + "\n")

    @watch_config_events.before_loop
    async def _before_watch_config_events(self):
        await self.wait_until_ready()

    @tasks.loop(minutes=15)
    async def watch_mods(self):
        manifest = await fetch_manifest()
        if manifest is None:
            return

        mods = manifest.get("mods", [])
        current = {
            m.get("id"): m
            for m in mods
            if m.get("id")
        }
        if self.known_mods is None:
            self.known_mods = current
            log.info("primed mod list (%d mods)", len(current))
            return

        previous = self.known_mods
        self.known_mods = current

        for mod_id, m in current.items():
            old = previous.get(mod_id)
            fingerprint = (
                m.get("version"),
                m.get("sha256"),
                m.get("gameVersion"),
                bool(m.get("enabled", True)),
            )
            old_fingerprint = None if old is None else (
                old.get("version"),
                old.get("sha256"),
                old.get("gameVersion"),
                bool(old.get("enabled", True)),
            )
            if old is not None and fingerprint == old_fingerprint:
                continue

            version = m.get("version", "?")
            enabled = bool(m.get("enabled", True))
            scope = "Server + client" if m.get("serverSide") else "Client only"
            release_url = f"{MODS_REPOSITORY}/releases/tag/{mod_id}-v{version}"

            if old is None:
                title = f"New guild mod: {m.get('name', mod_id)} v{version}"
                change = "A new mod is ready."
                colour = AMBER
            elif not enabled:
                title = f"Mod disabled: {m.get('name', mod_id)}"
                change = "Untick this mod in the manager and apply changes."
                colour = RED
            else:
                old_version = old.get("version", "?")
                title = f"Mod updated: {m.get('name', mod_id)} v{version}"
                change = f"Updated from v{old_version} to v{version}."
                colour = GREEN

            e = discord.Embed(
                title=title,
                description=m.get("description", ""),
                colour=colour,
                timestamp=datetime.now(timezone.utc),
                url=release_url,
            )
            e.add_field(name="Compatibility", value=m.get("gameVersion", "Not listed"))
            e.add_field(name="Install scope", value=scope)
            e.add_field(name="Deployment", value=change, inline=False)
            e.add_field(
                name="Install / update",
                value="Run **/getmods**, extract the ZIP, then run **Install Mods.bat**.",
                inline=False,
            )
            if m.get("notes"):
                e.add_field(name="Release note", value=m["notes"][:1024], inline=False)

            text = (
                f"**{title}**\n"
                f"{m.get('description', '')}\n"
                f"Compatibility: `{m.get('gameVersion', 'Not listed')}` | Scope: **{scope}**\n"
                f"{change}\n"
                f"Run **/getmods** for the current installer.\n"
                f"{release_url}"
            )
            await self.alert(e, text)

        for mod_id, old in previous.items():
            if mod_id in current:
                continue
            title = f"Mod removed: {old.get('name', mod_id)}"
            e = discord.Embed(
                title=title,
                description="This mod is no longer in the supported guild set.",
                colour=RED,
                timestamp=datetime.now(timezone.utc),
            )
            e.add_field(
                name="Action required",
                value="Run **/getmods**, then untick the removed mod and apply changes.",
                inline=False,
            )
            await self.alert(
                e,
                f"**{title}**\nRun **/getmods**, untick it, and apply changes.",
            )

    @watch_mods.before_loop
    async def _before_watch_mods(self):
        await self.wait_until_ready()

    async def on_interaction(self, interaction: discord.Interaction):
        # Diagnostic: proves whether slash commands are actually reaching the bot.
        try:
            name = interaction.data.get("name") if interaction.data else "?"
        except Exception:  # noqa: BLE001
            name = "?"
        log.info(
            "INTERACTION received: /%s from %s in #%s",
            name,
            getattr(interaction.user, "name", "?"),
            getattr(interaction.channel, "name", "?"),
        )

    async def on_ready(self):
        log.info("connected as %s", self.user)
        # Copy the commands into every guild we are in and sync there: guild-scoped
        # commands appear instantly, unlike the global ones.
        for g in self.guilds:
            try:
                # copy_global_to puts the locally-defined commands into this guild's
                # bucket; syncing the guild then registers them for instant use.
                self.tree.copy_global_to(guild=g)
                await self.tree.sync(guild=g)
                log.info("commands synced to guild %s (%s) - available now", g.name, g.id)
            except Exception as e:  # noqa: BLE001
                log.error("guild sync failed for %s: %s", g.id, e)
        await self.change_presence(
            activity=discord.Activity(type=discord.ActivityType.watching, name="the Palworld server")
        )


bot = PalBot()


@bot.tree.command(name="status", description="Live Palworld server status")
async def cmd_status(interaction: discord.Interaction):
    await interaction.response.defer()
    info = await bot.api.info()
    mx = await bot.api.metrics()
    if info is None:
        e = discord.Embed(
            title="Server unreachable",
            description="No answer from the Palworld server right now.",
            colour=RED,
        )
        await interaction.followup.send(embed=e)
        return

    e = discord.Embed(title=info.get("servername", "Palworld Server"), colour=GREEN)
    if info.get("version"):
        e.set_footer(text=f"v{info['version']}")
    bits = []
    if mx:
        e.add_field(name="Players", value=f"{mx.get('currentplayernum', '?')} / {mx.get('maxplayernum', '?')}")
        e.add_field(name="In-game day", value=str(mx.get("days", "?")))
        e.add_field(name="Uptime", value=human_uptime(mx.get("uptime")))
        fps = mx.get("serverfps")
        if fps is not None:
            e.add_field(name="Server FPS", value=str(fps))
        if mx.get("basecampnum") is not None:
            e.add_field(name="Base camps", value=str(mx["basecampnum"]))
        bits = [
            f"{mx.get('currentplayernum', '?')}/{mx.get('maxplayernum', '?')} online",
            f"day {mx.get('days', '?')}",
            f"up {human_uptime(mx.get('uptime'))}",
        ]
        if fps is not None:
            bits.append(f"{fps} FPS")
    last_change = load_last_config_change()
    if last_change:
        keys = sorted({str(key) for key in last_change.get("changed_keys", []) if key})
        if keys:
            changed_at = last_change.get("created_at", "")
            try:
                when = fmt_pacific(datetime.fromisoformat(changed_at.replace("Z", "+00:00")))
            except (TypeError, ValueError):
                when = "time unknown"
            summary = f"{', '.join(keys)}\n{when}"
            e.add_field(name="Last config change", value=summary[:1024], inline=False)
            bits.append(f"config changed {when}")
    # Plain-text mirror: some clients have embeds switched off entirely.
    text = f"**{info.get('servername', 'Palworld Server')}** - " + " | ".join(bits) if bits else "Server is up."
    await interaction.followup.send(content=text, embed=e)


@bot.tree.command(name="players", description="Who is online right now")
async def cmd_players(interaction: discord.Interaction):
    await interaction.response.defer()
    players = await bot.api.players()
    if players is None:
        await interaction.followup.send(
            embed=discord.Embed(title="Server unreachable", colour=RED)
        )
        return
    if not players:
        await interaction.followup.send(
            content="**Nobody online** - the island is quiet.",
            embed=discord.Embed(title="Nobody online", description="The island is quiet.", colour=GREY),
        )
        return

    e = discord.Embed(title=f"Online now ({len(players)})", colour=GREEN)
    lines = []
    for p in players[:25]:
        name = p.get("name") or p.get("accountName") or "unknown"
        bits = []
        if p.get("level") is not None:
            bits.append(f"Lv {p['level']}")
        if p.get("ping") is not None:
            bits.append(f"{round(float(p['ping']))} ms")
        if p.get("location_x") is not None and p.get("location_y") is not None:
            mx_, my_ = world_to_map(p["location_x"], p["location_y"])
            if mx_ is not None:
                bits.append(f"map {mx_:.0f}, {my_:.0f}")
        e.add_field(name=name, value=(" - ".join(bits) or "online"), inline=True)
        lines.append(f"- **{name}** {(' - '.join(bits))}".rstrip())
    text = f"**Online now ({len(players)})**\n" + "\n".join(lines)
    await interaction.followup.send(content=text[:1900], embed=e)


@bot.tree.command(name="mods", description="The guild mod set and how to install it")
async def cmd_mods(interaction: discord.Interaction):
    await interaction.response.defer()
    manifest = await fetch_manifest()
    if manifest is None:
        await interaction.followup.send(
            embed=discord.Embed(title="Could not reach the mod list", colour=RED)
        )
        return

    mods = manifest.get("mods", [])
    e = discord.Embed(
        title="Guild Mods",
        description="Run **/getmods** to download the self-installing pack, then tick what you want and Apply.",
        colour=AMBER,
    )
    e.add_field(
        name="PAL COMMAND companion",
        value="Use **/players** for live in-game map coordinates. Admins also get copyable coordinates in the Players dashboard.",
        inline=False,
    )
    lines = [
        "**Guild Mods** - run **/getmods** to download the self-installing pack.",
        "**PAL COMMAND:** use **/players** for live in-game map coordinates.",
    ]
    for m in mods:
        label = m.get("name", "?")
        if m.get("version"):
            label += f" (v{m['version']})"
        e.add_field(name=label, value=m.get("description", ""), inline=False)
        lines.append(f"- **{label}** - {m.get('description', '')}")
    await interaction.followup.send(content="\n".join(lines)[:1900], embed=e)


@bot.tree.command(name="backup", description="When the world was last backed up")
async def cmd_backup(interaction: discord.Interaction):
    await interaction.response.defer()
    s = load_settings()
    repo = s.get("repo_local_path", "")
    saves = pathlib.Path(repo) / "saves" if repo else None
    if not saves or not saves.is_dir():
        await interaction.followup.send(
            embed=discord.Embed(title="No backups found yet", colour=GREY)
        )
        return
    snaps = sorted(saves.glob("*.tar.gz"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not snaps:
        await interaction.followup.send(
            embed=discord.Embed(title="No snapshots yet", colour=GREY)
        )
        return
    newest = snaps[0]
    when = datetime.fromtimestamp(newest.stat().st_mtime)
    size_mb = newest.stat().st_size / 1_048_576
    offsite = bool(s.get("repo_remote"))
    e = discord.Embed(title="Backup status", colour=GREEN if offsite else AMBER)
    e.add_field(name="Last backup", value=fmt_pacific(when))
    e.add_field(name="Size", value=f"{size_mb:.1f} MB")
    e.add_field(name="Snapshots kept", value=str(len(snaps)))
    e.add_field(
        name="Off-site",
        value="Yes - pushed to private GitHub" if offsite else "No - local only",
        inline=False,
    )
    text = (
        f"**Backup status** - last {fmt_pacific(when)} | "
        f"{size_mb:.1f} MB | {len(snaps)} snapshots | "
        f"off-site: {'yes' if offsite else 'no'}"
    )
    await interaction.followup.send(content=text, embed=e)


@bot.tree.command(name="getmods", description="Get the current guild mod pack (self-installing zip)")
async def cmd_getmods(interaction: discord.Interaction):
    await interaction.response.defer()
    builder = os.path.join(MODS_REPO, "New-GuildBundle.ps1")
    if not os.path.isfile(builder):
        await interaction.followup.send(
            "The mod bundle builder isn't reachable on the host right now - ping the admin."
        )
        return
    out = os.path.join(tempfile.gettempdir(), "GuildMods.zip")
    try:
        proc = await asyncio.to_thread(
            subprocess.run,
            ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", builder, "-OutPath", out],
            capture_output=True, text=True, timeout=120,
        )
    except Exception as e:  # noqa: BLE001
        log.error("getmods build error: %s", e)
        await interaction.followup.send("Couldn't build the mod pack - ping the admin.")
        return
    if proc.returncode != 0 or not os.path.isfile(out):
        log.error("getmods build failed rc=%s: %s", proc.returncode, proc.stderr[:500])
        await interaction.followup.send("Couldn't build the mod pack - ping the admin.")
        return

    # Discord attachment limit (25 MB on most servers); our bundles are tiny, but guard anyway.
    if os.path.getsize(out) > 24 * 1024 * 1024:
        await interaction.followup.send("The mod pack is too big to attach here - ping the admin.")
        return

    msg = (
        "**Palworld Guild Mods**\n"
        "1. Download **GuildMods.zip** below and unzip it.\n"
        "2. Double-click **Install Mods.bat**.\n"
        "3. Tick the mods you want, press **Apply Changes**.\n"
        "_Close Palworld first. Steam version only._"
    )
    await interaction.followup.send(content=msg, file=discord.File(out, filename="GuildMods.zip"))


# ---------------------------------------------------------------- guarded restart

RESTART_COOLDOWN_SEC = 30 * 60          # anti-spam: at most one restart per 30 min
_restart_lock = asyncio.Lock()          # no two restarts running at once
_last_restart_file = CFG_DIR / "auto" / "last_restart.txt"
BACKUP_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "palcommand-backup.ps1")


def _restart_cooldown_remaining() -> int:
    try:
        ts = float(_last_restart_file.read_text().strip())
    except Exception:  # noqa: BLE001
        return 0
    return max(0, int(RESTART_COOLDOWN_SEC - (time.time() - ts)))


def _mark_restart() -> None:
    try:
        _last_restart_file.parent.mkdir(parents=True, exist_ok=True)
        _last_restart_file.write_text(str(time.time()))
    except Exception as e:  # noqa: BLE001
        log.error("could not record restart time: %s", e)


@bot.tree.command(name="restart", description="Gracefully restart the server (warns players, backs up first)")
async def cmd_restart(interaction: discord.Interaction):
    remaining = _restart_cooldown_remaining()
    if remaining > 0:
        await interaction.response.send_message(
            f"⏳ Restart is on cooldown - try again in {remaining // 60}m {remaining % 60}s.")
        return
    if _restart_lock.locked():
        await interaction.response.send_message("🔄 A restart is already running - hang tight.")
        return

    await interaction.response.defer()
    async with _restart_lock:
        if _restart_cooldown_remaining() > 0:
            await interaction.followup.send("A restart just ran - please wait for the cooldown.")
            return

        info = await bot.api.info()
        if info is None:
            await interaction.followup.send(
                "The server isn't answering right now. If it's down it should auto-recover shortly; "
                "otherwise an admin can hit Restart in the Host Havoc panel.")
            return

        # Burn the cooldown up front so the button can't be double-fired mid-flow.
        _mark_restart()
        msg = await interaction.followup.send("**Guarded restart** - starting…", wait=True)

        async def step(text: str):
            try:
                await msg.edit(content=f"**Guarded restart**\n{text}")
            except Exception:  # noqa: BLE001
                pass

        try:
            players = await bot.api.players() or []

            # 1) Warn players in-game with a countdown (this is the "graceful" part).
            if players:
                await step(f"⚠️ {len(players)} player(s) online — warning them, restarting in 60s…")
                for announce_at, sleep_for in ((60, 30), (30, 20), (10, 10)):
                    await bot.api.announce(f"Server restart in {announce_at} seconds - get somewhere safe!")
                    await asyncio.sleep(sleep_for)
            else:
                await step("No players online — proceeding.")
                await bot.api.announce("Server restarting shortly for a scheduled backup.")
                await asyncio.sleep(3)

            # 2) Verified, off-site backup FIRST (never restart without a safe snapshot).
            await step("💾 Creating a verified off-site backup…")
            proc = await asyncio.to_thread(
                subprocess.run,
                ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", BACKUP_SCRIPT, "-Quiet"],
                capture_output=True, text=True, timeout=300,
            )
            if proc.returncode != 0:
                await step("❌ Backup FAILED — restart aborted, server left running. Ping the admin.")
                log.error("restart backup failed rc=%s: %s", proc.returncode, (proc.stderr or "")[:500])
                return

            # 3) Restart. Host Havoc's watchdog brings it back after shutdown.
            await bot.api.announce("Backup complete - restarting now!")
            await step("🔁 Backup verified. Restarting the server…")
            await bot.api.shutdown(10, "Server restarting now")

            # 4) Confirm recovery: wait for it to drop, then come back.
            start = time.monotonic()
            saw_down = False
            recovered = None
            while time.monotonic() - start < 180:
                await asyncio.sleep(5)
                up = (await bot.api.info()) is not None
                if not up:
                    saw_down = True
                elif saw_down:
                    recovered = int(time.monotonic() - start)
                    break

            if recovered is not None:
                await step(f"✅ Back online — recovered in {recovered}s. Verified backup is safe off-site.")
            else:
                await step(
                    "⚠️ Backup is safe and shutdown was sent, but recovery wasn't confirmed within 180s. "
                    "It may still be booting — give it a minute, or an admin can check the panel.")
        except Exception as e:  # noqa: BLE001
            log.error("restart flow error: %s", e)
            await step(f"❌ Restart hit an error: {e}. Your backup may still be safe — check with the admin.")


def main():
    if not TOKEN:
        print("ERROR: no Discord token. Launch this with Start-PalBot.ps1, which decrypts it.")
        sys.exit(1)
    if not CHANNEL_ID:
        print("WARNING: no alert channel set - slash commands will work, alerts will not.")
    try:
        bot.run(TOKEN, log_handler=None)
    except discord.LoginFailure:
        print("ERROR: Discord rejected the bot token. Re-run -SetupDiscord with a fresh token.")
        sys.exit(1)


if __name__ == "__main__":
    main()
