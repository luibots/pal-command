#!/usr/bin/env python3
"""Add a megabase material package to one guild's shared chest offline."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path


MAX_STACK = 9999
PACKAGE_MINIMUMS = {
    "Wood_Fine": 29999,
    "Stone": 19999,
    "Pal_crystal_S": 19999,
    "Wood": 9999,
    "Quartz": 9999,
    "IronIngot": 9999,
    "StealIngot": 9999,
    "Cement": 9999,
    "Polymer": 9999,
    "CarbonFiber": 9999,
    "MachineParts2": 5000,
    "CrudeOil": 5000,
    "PalCrystal_Ex": 2000,
    "AncientParts2": 500,
    "Plastic": 5000,
    "StainlessSteel": 5000,
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def configure_imports(tool_root: Path) -> None:
    sys.path.insert(0, str(tool_root / "src"))
    sys.path.insert(0, str(tool_root / "src" / "palsav"))
    sys.path.insert(0, str(tool_root / "src" / "palsav" / "palooz"))


def clean_uid(value: object) -> str:
    return str(value).replace("-", "").lower()


def find_player_guild(level, player_uid: str) -> tuple[str, str]:
    wanted = clean_uid(player_uid)
    world = level["properties"]["worldSaveData"]["value"]
    for group in world.get("GroupSaveDataMap", {}).get("value", []):
        raw = group.get("value", {}).get("RawData", {}).get("value", {})
        for player in raw.get("players", []):
            if clean_uid(player.get("player_uid")) == wanted:
                name = player.get("player_info", {}).get("player_name", "")
                group_type = (
                    group.get("value", {})
                    .get("GroupType", {})
                    .get("value", {})
                    .get("value", "")
                )
                if group_type != "EPalGroupType::Guild":
                    raise RuntimeError(f"Player {name or player_uid} is not in a guild")
                return str(group["key"]), name
    raise RuntimeError(f"Player {player_uid} is absent from Level.sav")


def container_totals(container) -> dict[str, int]:
    totals: dict[str, int] = {}
    for item in container.get_items():
        item_id = item["item_id"]
        totals[item_id] = totals.get(item_id, 0) + int(item["stack_count"])
    return totals


def canonical_hash(value: object) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), default=str
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def other_container_hash(level, target_id: str) -> str:
    wanted = clean_uid(target_id)
    world = level["properties"]["worldSaveData"]["value"]
    containers = []
    for entry in world.get("ItemContainerSaveData", {}).get("value", []):
        entry_id = clean_uid(
            entry.get("key", {}).get("ID", {}).get("value", "")
        )
        if entry_id != wanted:
            containers.append(entry)
    return canonical_hash(containers)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--save-dir", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--player-uid", required=True)
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    save_dir = args.save_dir.resolve()
    tool_root = args.tool_root.resolve()
    level_path = save_dir / "Level.sav"
    if not level_path.is_file():
        raise RuntimeError(f"Missing Level.sav: {level_path}")

    configure_imports(tool_root)
    os.chdir(tool_root)

    from palworld_aio import constants
    from palworld_aio.inventory.base_inventory_manager import get_guild_chest
    from palworld_aio.inventory.inventory_manager import InventoryContainer
    from palworld_aio.utils import sav_to_gvas_wrapper, wrapper_to_sav

    player_hashes_before = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    level = sav_to_gvas_wrapper(str(level_path))
    constants.current_save_path = str(save_dir)
    constants.loaded_level_json = level
    constants.invalidate_container_lookup()

    guild_id, player_name = find_player_guild(level, args.player_uid)
    guild_chest = get_guild_chest(guild_id)
    if not guild_chest:
        raise RuntimeError(
            f"Guild Chest is missing for {player_name or args.player_uid}'s guild"
        )

    container_id = guild_chest["id"]
    lookup = constants.get_container_lookup()
    container_data = lookup.get(clean_uid(container_id))
    if not container_data:
        raise RuntimeError("Guild Chest inventory container is missing")
    slot_count = int(
        container_data.get("value", {}).get("SlotNum", {}).get("value", 0)
    )
    if slot_count <= 0:
        raise RuntimeError("Guild Chest reports zero storage slots")

    container = InventoryContainer(
        container_id, container_data, max_slots=slot_count, container_type="GuildChest"
    )
    totals_before = container_totals(container)
    empty_slots = sum(
        1
        for slot in container._standardized_container.slots
        if not slot.item_id
    )
    chunks_needed: dict[str, list[int]] = {}
    for item_id, target in PACKAGE_MINIMUMS.items():
        deficit = max(0, target - totals_before.get(item_id, 0))
        chunks = []
        while deficit:
            chunk = min(MAX_STACK, deficit)
            chunks.append(chunk)
            deficit -= chunk
        chunks_needed[item_id] = chunks
    new_slots_required = sum(len(chunks) for chunks in chunks_needed.values())

    report = {
        "dry_run": args.dry_run,
        "player": player_name,
        "guild_id": guild_id,
        "guild_chest_id": container_id,
        "slot_count": slot_count,
        "empty_slots": empty_slots,
        "new_slots_required": new_slots_required,
        "package_minimums": PACKAGE_MINIMUMS,
        "totals_before": {
            item_id: totals_before.get(item_id, 0)
            for item_id in PACKAGE_MINIMUMS
        },
    }
    if empty_slots < new_slots_required:
        raise RuntimeError(
            f"Guild Chest needs {new_slots_required} free slots but has {empty_slots}"
        )
    if args.dry_run:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0

    other_hash_before = other_container_hash(level, container_id)
    for item_id, chunks in chunks_needed.items():
        for chunk in chunks:
            if not container.add_item(item_id, chunk):
                raise RuntimeError(f"Could not add {chunk} x {item_id}")
    container_data["value"]["Slots"]["value"]["values"] = (
        container._standardized_container.get_raw_slots()
    )
    wrapper_to_sav(level, str(level_path))

    # Reopen and verify the exact target plus unrelated-container/player invariants.
    verified_level = sav_to_gvas_wrapper(str(level_path))
    constants.loaded_level_json = verified_level
    constants.invalidate_container_lookup()
    verified_chest = get_guild_chest(guild_id)
    if not verified_chest or clean_uid(verified_chest["id"]) != clean_uid(container_id):
        raise RuntimeError("Guild Chest identity changed after serialization")
    verified_data = constants.get_container_lookup().get(clean_uid(container_id))
    verified_container = InventoryContainer(
        container_id,
        verified_data,
        max_slots=slot_count,
        container_type="GuildChest",
    )
    totals_after = container_totals(verified_container)
    for item_id, target in PACKAGE_MINIMUMS.items():
        if totals_after.get(item_id, 0) < target:
            raise RuntimeError(
                f"{item_id} verification failed: {totals_after.get(item_id, 0)} < {target}"
            )
    if other_container_hash(verified_level, container_id) != other_hash_before:
        raise RuntimeError("An unrelated item container changed")

    player_hashes_after = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    if player_hashes_after != player_hashes_before:
        raise RuntimeError("One or more player profile files changed")

    report.update(
        {
            "totals_after": {
                item_id: totals_after.get(item_id, 0)
                for item_id in PACKAGE_MINIMUMS
            },
            "unrelated_containers_changed": False,
            "player_profiles_changed": False,
        }
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
