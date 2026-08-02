#!/usr/bin/env python3
"""Fill one explicitly identified ordinary chest with a level-54 build package."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import sys
from pathlib import Path


MAX_STACK = 9999
PACKAGE_MINIMUMS = {
    "Wood_Fine": 29999,
    "Stone": 29999,
    "Pal_crystal_S": 29999,
    "Wood": 9999,
    "Quartz": 9999,
    "CopperOre": 9999,
    "Coal": 9999,
    "Sulfur": 5000,
    "CopperIngot": 9999,
    "IronIngot": 9999,
    "StealIngot": 9999,
    "Cement": 9999,
    "Polymer": 9999,
    "CarbonFiber": 9999,
    "MachineParts2": 5000,
    "CrudeOil": 5000,
    "PalCrystal_Ex": 2000,
    "PalFluid": 5000,
    "PalOil": 5000,
    "ElectricOrgan": 5000,
    "FireOrgan": 5000,
    "IceOrgan": 5000,
    "Leather": 5000,
    "bone": 5000,
    "Fiber": 9999,
    "Cloth2": 5000,
}


def clean_uid(value: object) -> str:
    return str(value).replace("-", "").lower()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_hash(value: object) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), default=str
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def other_container_hash(level, target_id: str) -> str:
    wanted = clean_uid(target_id)
    world = level["properties"]["worldSaveData"]["value"]
    others = []
    for entry in world.get("ItemContainerSaveData", {}).get("value", []):
        entry_id = clean_uid(entry.get("key", {}).get("ID", {}).get("value", ""))
        if entry_id != wanted:
            others.append(entry)
    return canonical_hash(others)


def container_totals(container) -> dict[str, int]:
    totals: dict[str, int] = {}
    for item in container.get_items():
        item_id = item["item_id"]
        if item_id:
            totals[item_id] = totals.get(item_id, 0) + int(item["stack_count"])
    return totals


def find_linked_chest(level, container_id: str) -> dict[str, str]:
    wanted = clean_uid(container_id)
    world = level["properties"]["worldSaveData"]["value"]
    matches = []
    for obj in world.get("MapObjectSaveData", {}).get("value", {}).get("values", []):
        for module in (
            obj.get("ConcreteModel", {})
            .get("value", {})
            .get("ModuleMap", {})
            .get("value", [])
        ):
            if module.get("key") != "EPalMapObjectConcreteModelModuleType::ItemContainer":
                continue
            raw = module.get("value", {}).get("RawData", {}).get("value", {})
            if clean_uid(raw.get("target_container_id")) != wanted:
                continue
            model_raw = (
                obj.get("Model", {}).get("value", {}).get("RawData", {}).get("value", {})
            )
            matches.append(
                {
                    "map_object_id": str(obj.get("MapObjectId", {}).get("value", "")),
                    "base_id": str(model_raw.get("base_camp_id_belong_to", "")),
                    "guild_id": str(model_raw.get("group_id_belong_to", "")),
                    "builder_uid": str(model_raw.get("build_player_uid", "")),
                    "instance_id": str(model_raw.get("instance_id", "")),
                }
            )
    if len(matches) != 1:
        raise RuntimeError(f"Expected one linked map object, found {len(matches)}")
    return matches[0]


def configure_imports(tool_root: Path) -> None:
    sys.path.insert(0, str(tool_root / "src"))
    sys.path.insert(0, str(tool_root / "src" / "palsav"))
    sys.path.insert(0, str(tool_root / "src" / "palsav" / "palooz"))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--save-dir", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--container-id", required=True)
    parser.add_argument("--base-id", required=True)
    parser.add_argument("--guild-id", required=True)
    parser.add_argument("--builder-uid", required=True)
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
    from palworld_aio.inventory.inventory_manager import InventoryContainer
    from palworld_aio.utils import sav_to_gvas_wrapper, wrapper_to_sav

    player_hashes_before = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    level = sav_to_gvas_wrapper(str(level_path))
    constants.current_save_path = str(save_dir)
    constants.loaded_level_json = level
    constants.invalidate_container_lookup()

    link = find_linked_chest(level, args.container_id)
    expected = {
        "map_object_id": "ItemChest_03",
        "base_id": args.base_id,
        "guild_id": args.guild_id,
        "builder_uid": args.builder_uid,
    }
    for key, value in expected.items():
        if clean_uid(link[key]) != clean_uid(value):
            raise RuntimeError(f"Target {key} mismatch: {link[key]} != {value}")

    lookup = constants.get_container_lookup()
    container_data = lookup.get(clean_uid(args.container_id))
    if not container_data:
        raise RuntimeError("Target inventory container is missing")
    slot_count = int(container_data.get("value", {}).get("SlotNum", {}).get("value", 0))
    if slot_count != 40:
        raise RuntimeError(f"Expected a 40-slot chest, found {slot_count} slots")

    container = InventoryContainer(
        args.container_id,
        container_data,
        max_slots=slot_count,
        container_type="ItemChest",
    )
    totals_before = container_totals(container)
    if totals_before != {"MachineParts2": 5}:
        raise RuntimeError(
            "Marker guard failed; expected the stash to contain only 5 circuit boards, "
            f"found {totals_before}"
        )

    chunks_needed: dict[str, list[int]] = {}
    for item_id, target in PACKAGE_MINIMUMS.items():
        deficit = max(0, target - totals_before.get(item_id, 0))
        chunks = []
        while deficit:
            chunks.append(min(MAX_STACK, deficit))
            deficit -= chunks[-1]
        chunks_needed[item_id] = chunks

    # Simulate against a deep copy so dry-run verifies actual merge and slot behavior.
    simulated_data = copy.deepcopy(container_data)
    simulated = InventoryContainer(
        args.container_id,
        simulated_data,
        max_slots=slot_count,
        container_type="ItemChest",
    )
    for item_id, chunks in chunks_needed.items():
        for chunk in chunks:
            if not simulated.add_item(item_id, chunk):
                raise RuntimeError(f"Chest capacity is insufficient for {chunk} x {item_id}")
    totals_simulated = container_totals(simulated)
    occupied_after = sum(
        1 for slot in simulated._standardized_container.slots if slot.item_id
    )

    report = {
        "dry_run": args.dry_run,
        "target": {"container_id": args.container_id, **link},
        "slot_count": slot_count,
        "occupied_after": occupied_after,
        "free_after": slot_count - occupied_after,
        "totals_before": totals_before,
        "package_minimums": PACKAGE_MINIMUMS,
    }
    if args.dry_run:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0

    other_hash_before = other_container_hash(level, args.container_id)
    for item_id, chunks in chunks_needed.items():
        for chunk in chunks:
            if not container.add_item(item_id, chunk):
                raise RuntimeError(f"Could not add {chunk} x {item_id}")
    container_data["value"]["Slots"]["value"]["values"] = (
        container._standardized_container.get_raw_slots()
    )
    wrapper_to_sav(level, str(level_path))

    verified_level = sav_to_gvas_wrapper(str(level_path))
    constants.loaded_level_json = verified_level
    constants.invalidate_container_lookup()
    verified_link = find_linked_chest(verified_level, args.container_id)
    if verified_link != link:
        raise RuntimeError("Target map-object identity changed after serialization")
    verified_data = constants.get_container_lookup().get(clean_uid(args.container_id))
    verified = InventoryContainer(
        args.container_id,
        verified_data,
        max_slots=slot_count,
        container_type="ItemChest",
    )
    totals_after = container_totals(verified)
    for item_id, target in PACKAGE_MINIMUMS.items():
        if totals_after.get(item_id, 0) < target:
            raise RuntimeError(
                f"{item_id} verification failed: {totals_after.get(item_id, 0)} < {target}"
            )
    if other_container_hash(verified_level, args.container_id) != other_hash_before:
        raise RuntimeError("An unrelated item container changed")

    player_hashes_after = {
        path.name: sha256(path) for path in (save_dir / "Players").glob("*.sav")
    }
    if player_hashes_after != player_hashes_before:
        raise RuntimeError("One or more player profile files changed")

    report.update(
        {
            "totals_after": {
                item_id: totals_after.get(item_id, 0) for item_id in PACKAGE_MINIMUMS
            },
            "unrelated_containers_changed": False,
            "player_profiles_changed": False,
        }
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
