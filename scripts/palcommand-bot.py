"""
palcommand-bot.py - Discord bot for the PAL COMMAND Palworld server.

Slash commands:
    /status    live server state (players, day, FPS, uptime)
    /players   who is online right now
    /mods      the guild mod set + how to install it
    /backup    when the last backup ran and whether it is off-site

Background monitors (post to the alert channel on change):
    * server up/down  - polled every 2 minutes, alerts only on state change
    * new mod         - polls the public mods.json, announces newly published mods

Secrets are never stored here. Start-PalBot.ps1 decrypts them from DPAPI and
passes them in as environment variables.
"""

import asyncio
import json
import logging
import os
import pathlib
import sys
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

AMBER = 0xF5A524
GREEN = 0x22C55E
RED = 0xEF4444
GREY = 0x6B7280

CFG_DIR = pathlib.Path(os.environ["APPDATA"]) / "com.luibots.palcommand"
SETTINGS = CFG_DIR / "settings.json"


def load_settings() -> dict:
    try:
        # settings.json may carry a BOM (written by PowerShell)
        return json.loads(SETTINGS.read_text(encoding="utf-8-sig"))
    except Exception as e:  # noqa: BLE001
        log.error("could not read settings.json: %s", e)
        return {}


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


class PalBot(discord.Client):
    def __init__(self):
        intents = discord.Intents.default()
        super().__init__(intents=intents)
        self.tree = app_commands.CommandTree(self)
        self.settings = load_settings()
        self.api = PalAPI(self.settings.get("rest_url", ""), ADMIN_PW)
        self.last_up = None          # None = unknown yet
        self.known_mod_ids = None    # None = not primed yet

    async def setup_hook(self):
        # Global sync makes the commands available everywhere eventually, but Discord
        # caches global commands for up to an hour. The per-guild sync in on_ready is
        # what makes them show up immediately.
        await self.tree.sync()
        log.info("slash commands synced globally")
        self.watch_server.start()
        self.watch_mods.start()

    async def alert(self, embed: discord.Embed):
        if not CHANNEL_ID:
            return
        ch = self.get_channel(CHANNEL_ID)
        if ch is None:
            try:
                ch = await self.fetch_channel(CHANNEL_ID)
            except Exception as e:  # noqa: BLE001
                log.error("alert channel %s unreachable: %s", CHANNEL_ID, e)
                return
        try:
            await ch.send(embed=embed)
        except Exception as e:  # noqa: BLE001
            log.error("could not post alert: %s", e)

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

    @tasks.loop(minutes=15)
    async def watch_mods(self):
        try:
            timeout = aiohttp.ClientTimeout(total=15)
            async with aiohttp.ClientSession(timeout=timeout) as s:
                async with s.get(MODS_MANIFEST) as r:
                    if r.status != 200:
                        return
                    manifest = await r.json(content_type=None)
        except Exception as e:  # noqa: BLE001
            log.debug("mods manifest fetch failed: %s", e)
            return

        mods = manifest.get("mods", [])
        ids = {m.get("id") for m in mods}
        if self.known_mod_ids is None:
            self.known_mod_ids = ids
            log.info("primed mod list (%d mods)", len(ids))
            return
        new = ids - self.known_mod_ids
        self.known_mod_ids = ids
        for m in mods:
            if m.get("id") in new:
                e = discord.Embed(
                    title=f"New mod available: {m.get('name')}",
                    description=m.get("description", ""),
                    colour=AMBER,
                    timestamp=datetime.now(timezone.utc),
                )
                e.add_field(
                    name="How to get it",
                    value="Open your **Palworld Mod Manager**, tick the mod, press **Apply Changes**.",
                    inline=False,
                )
                if m.get("notes"):
                    e.add_field(name="Note", value=m["notes"], inline=False)
                await self.alert(e)

    @watch_mods.before_loop
    async def _before_watch_mods(self):
        await self.wait_until_ready()

    async def on_ready(self):
        log.info("connected as %s", self.user)
        # Copy the commands into every guild we are in and sync there: guild-scoped
        # commands appear instantly, unlike the global ones.
        for g in self.guilds:
            try:
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
    if mx:
        e.add_field(name="Players", value=f"{mx.get('currentplayernum', '?')} / {mx.get('maxplayernum', '?')}")
        e.add_field(name="In-game day", value=str(mx.get("days", "?")))
        e.add_field(name="Uptime", value=human_uptime(mx.get("uptime")))
        fps = mx.get("serverfps")
        if fps is not None:
            e.add_field(name="Server FPS", value=str(fps))
        if mx.get("basecampnum") is not None:
            e.add_field(name="Base camps", value=str(mx["basecampnum"]))
    await interaction.followup.send(embed=e)


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
            embed=discord.Embed(title="Nobody online", description="The island is quiet.", colour=GREY)
        )
        return

    e = discord.Embed(title=f"Online now ({len(players)})", colour=GREEN)
    for p in players[:25]:
        name = p.get("name") or p.get("accountName") or "unknown"
        bits = []
        if p.get("level") is not None:
            bits.append(f"Lv {p['level']}")
        if p.get("ping") is not None:
            bits.append(f"{round(float(p['ping']))} ms")
        e.add_field(name=name, value=(" - ".join(bits) or "online"), inline=True)
    await interaction.followup.send(embed=e)


@bot.tree.command(name="mods", description="The guild mod set and how to install it")
async def cmd_mods(interaction: discord.Interaction):
    await interaction.response.defer()
    try:
        timeout = aiohttp.ClientTimeout(total=15)
        async with aiohttp.ClientSession(timeout=timeout) as s:
            async with s.get(MODS_MANIFEST) as r:
                manifest = await r.json(content_type=None)
    except Exception:  # noqa: BLE001
        await interaction.followup.send(
            embed=discord.Embed(title="Could not reach the mod list", colour=RED)
        )
        return

    mods = manifest.get("mods", [])
    e = discord.Embed(
        title="Guild Mods",
        description="Download **Palworld Mod Manager.bat** from the mods page, double-click it, tick what you want, press Apply.",
        colour=AMBER,
    )
    for m in mods:
        e.add_field(name=m.get("name", "?"), value=m.get("description", ""), inline=False)
    e.add_field(name="Mods page", value="https://github.com/luibots/palworld-mods", inline=False)
    await interaction.followup.send(embed=e)


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
    e.add_field(name="Last backup", value=when.strftime("%Y-%m-%d %H:%M"))
    e.add_field(name="Size", value=f"{size_mb:.1f} MB")
    e.add_field(name="Snapshots kept", value=str(len(snaps)))
    e.add_field(
        name="Off-site",
        value="Yes - pushed to private GitHub" if offsite else "No - local only",
        inline=False,
    )
    await interaction.followup.send(embed=e)


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
