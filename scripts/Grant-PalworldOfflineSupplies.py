#!/usr/bin/env python3
"""Apply a narrowly scoped, offline Palworld player grant."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path


DEFAULT_ITEMS = {
    "Wood_Fine": 9999,
    "Stone": 9999,
    "Pal_crystal_S": 9999,
    "PalSphere_Ancient_2": 999,
    "PalSphere_Ancient_1": 999,
    "PalSphere_Exotic": 999,
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


def nested_value(value, default=0):
    while isinstance(value, dict) and "value" in value:
        value = value["value"]
    return default if value is None else value


def player_progress(constants, uid_clean: str) -> tuple[int, int]:
    entry = constants.player_character_cache.get(uid_clean.lower())
    if not entry:
        raise RuntimeError(f"Player {uid_clean} is absent from Level.sav")
    save_parameter = entry["value"]["RawData"]["value"]["object"]["SaveParameter"]["value"]
    return (
        int(nested_value(save_parameter.get("Level"), 0)),
        int(nested_value(save_parameter.get("Exp"), 0)),
    )


def item_totals(container) -> dict[str, int]:
    totals: dict[str, int] = {}
    for item in container.get_items():
        item_id = item["item_id"]
        totals[item_id] = totals.get(item_id, 0) + int(item["stack_count"])
    return totals


def ensure_item_minimum(container, item_id: str, target: int) -> None:
    matches = [item for item in container.get_items() if item["item_id"] == item_id]
    current = sum(int(item["stack_count"]) for item in matches)
    if current >= target:
        return
    deficit = target - current
    if matches:
        first = matches[0]
        if not container.set_item_count(first["slot_index"], int(first["stack_count"]) + deficit):
            raise RuntimeError(f"Could not update {item_id}")
    elif not container.add_item(item_id, target):
        raise RuntimeError(f"No free main-inventory slot for {item_id}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--save-dir", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--player-uid", required=True)
    parser.add_argument("--tech-points", type=int, default=999)
    parser.add_argument("--ancient-tech-points", type=int, default=999)
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    save_dir = args.save_dir.resolve()
    tool_root = args.tool_root.resolve()
    level_path = save_dir / "Level.sav"
    uid_clean = args.player_uid.replace("-", "").upper()
    player_path = save_dir / "Players" / f"{uid_clean}.sav"

    if not level_path.is_file() or not player_path.is_file():
        raise RuntimeError("Level.sav or the target player save is missing")
    if args.tech_points < 0 or args.ancient_tech_points < 0:
        raise RuntimeError("Technology points cannot be negative")

    configure_imports(tool_root)
    os.chdir(tool_root)

    from palworld_aio import constants
    from palworld_aio.inventory.inventory_manager import PlayerInventory
    from palworld_aio.managers.save_manager import build_player_levels
    from palworld_aio.utils import (
        gvasfile_to_sav,
        sav_to_gvas_wrapper,
        wrapper_to_sav,
    )

    player_hashes_before = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    level = sav_to_gvas_wrapper(str(level_path))
    constants.current_save_path = str(save_dir)
    constants.loaded_level_json = level
    constants.invalidate_container_lookup()
    build_player_levels()

    level_before, exp_before = player_progress(constants, uid_clean)
    inventory = PlayerInventory(args.player_uid)
    if not inventory.load():
        raise RuntimeError("Could not load the target player's inventory")
    main_container = inventory.get_container("main")
    if not main_container:
        raise RuntimeError("Target player's main inventory container is missing")

    totals_before = item_totals(main_container)
    empty_slots = sum(
        1
        for slot in main_container._standardized_container.slots
        if not slot.item_id
    )
    missing_stacks = sum(
        1
        for item_id, target in DEFAULT_ITEMS.items()
        if totals_before.get(item_id, 0) < target
        and not any(item["item_id"] == item_id for item in main_container.get_items())
    )

    player_save_data = inventory.player_gvas.properties["SaveData"]["value"]
    report = {
        "player_uid": args.player_uid,
        "level_before": level_before,
        "exp_before": exp_before,
        "tech_points_before": int(
            nested_value(player_save_data.get("TechnologyPoint"), 0)
        ),
        "ancient_tech_points_before": int(
            nested_value(player_save_data.get("bossTechnologyPoint"), 0)
        ),
        "empty_main_slots": empty_slots,
        "new_slots_required": missing_stacks,
        "requested_items": DEFAULT_ITEMS,
        "dry_run": args.dry_run,
    }

    if empty_slots < missing_stacks:
        raise RuntimeError(
            f"Need {missing_stacks} free inventory slots but only {empty_slots} are available"
        )
    if args.dry_run:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0

    for item_id, target in DEFAULT_ITEMS.items():
        ensure_item_minimum(main_container, item_id, target)
    if not inventory.save():
        raise RuntimeError("Could not synchronize inventory changes into Level.sav")

    player_save_data.setdefault(
        "TechnologyPoint", {"id": None, "value": 0, "type": "IntProperty"}
    )["value"] = args.tech_points
    player_save_data.setdefault(
        "bossTechnologyPoint", {"id": None, "value": 0, "type": "IntProperty"}
    )["value"] = args.ancient_tech_points

    gvasfile_to_sav(inventory.player_gvas, str(player_path))
    wrapper_to_sav(level, str(level_path))

    # Reopen the serialized files and verify the exact invariants we promised.
    constants.loaded_level_json = sav_to_gvas_wrapper(str(level_path))
    constants.invalidate_container_lookup()
    build_player_levels()
    level_after, exp_after = player_progress(constants, uid_clean)
    if (level_after, exp_after) != (level_before, exp_before):
        raise RuntimeError(
            f"Level/XP changed unexpectedly: {(level_before, exp_before)} -> "
            f"{(level_after, exp_after)}"
        )

    verified_inventory = PlayerInventory(args.player_uid)
    if not verified_inventory.load():
        raise RuntimeError("Modified inventory could not be reopened")
    verified_main = verified_inventory.get_container("main")
    totals_after = item_totals(verified_main)
    for item_id, target in DEFAULT_ITEMS.items():
        if totals_after.get(item_id, 0) < target:
            raise RuntimeError(
                f"{item_id} verification failed: {totals_after.get(item_id, 0)} < {target}"
            )

    verified_save_data = verified_inventory.player_gvas.properties["SaveData"]["value"]
    tech_after = int(nested_value(verified_save_data.get("TechnologyPoint"), -1))
    ancient_after = int(
        nested_value(verified_save_data.get("bossTechnologyPoint"), -1)
    )
    if (tech_after, ancient_after) != (
        args.tech_points,
        args.ancient_tech_points,
    ):
        raise RuntimeError(
            f"Technology-point verification failed: {(tech_after, ancient_after)}"
        )

    player_hashes_after = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    unrelated_changes = [
        name
        for name, digest in player_hashes_before.items()
        if name != player_path.name and player_hashes_after.get(name) != digest
    ]
    if unrelated_changes:
        raise RuntimeError(f"Unrelated player saves changed: {unrelated_changes}")

    report.update(
        {
            "level_after": level_after,
            "exp_after": exp_after,
            "tech_points_after": tech_after,
            "ancient_tech_points_after": ancient_after,
            "item_totals_after": {
                item_id: totals_after.get(item_id, 0) for item_id in DEFAULT_ITEMS
            },
            "unrelated_player_saves_changed": [],
        }
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
