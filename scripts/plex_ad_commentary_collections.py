#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""
plex_ad_commentary_collections.py
---------------------------------
Scan a Plex library and maintain static collections for movies/shows that
contain Audio Description (AD) or Commentary audio tracks.

AD detection  : Uses Plex's `visualImpaired` boolean flag on AudioStream
                (mapped from the MKV "visual impaired" track flag).
Commentary    : Plex does NOT expose the MKV "commentary" flag natively.
                We fall back to a case-insensitive keyword search in the
                track title / extendedDisplayTitle.

Requirements:
    pip install plexapi

Usage:
    # Dry-run (preview only, no changes):
    python plex_ad_commentary_collections.py --dry-run

    # Actually create/update collections:
    python plex_ad_commentary_collections.py

    # Custom collection names & library:
    python plex_ad_commentary_collections.py \
        --library "4K Movies" \
        --ad-collection "Audio Described" \
        --commentary-collection "Director Commentary"

    # TV shows:
    python plex_ad_commentary_collections.py --library "TV Shows"

    # Only AD (skip commentary scan):
    python plex_ad_commentary_collections.py --no-commentary

    # Only commentary (skip AD scan):
    python plex_ad_commentary_collections.py --no-ad

Environment variables (or pass via CLI flags):
    PLEX_URL    - e.g. http://localhost:32400
    PLEX_TOKEN  - your X-Plex-Token
"""

from __future__ import annotations

import argparse
import os
import sys
import time
from dataclasses import dataclass, field

try:
    from plexapi.server import PlexServer
    from plexapi.exceptions import NotFound
except ImportError:
    print("ERROR: python-plexapi is required.  Install with:  pip install plexapi")
    sys.exit(1)


import configparser

# -- Defaults ----------------------------------------------------------------

DEFAULT_PLEX_URL = "http://localhost:32400"
DEFAULT_LIBRARY = "Movies"
DEFAULT_AD_COLLECTION = "Audio Description (Film)"
DEFAULT_COMMENTARY_COLLECTION = "Commentary"

config_path = os.path.join(
    os.path.dirname(
        os.path.abspath(__file__)),
    "config.ini")
if os.path.exists(config_path):
    config = configparser.ConfigParser()
    config.read(config_path)
    if "PLEX" in config:
        DEFAULT_PLEX_URL = config["PLEX"].get("URL", DEFAULT_PLEX_URL)
        DEFAULT_LIBRARY = config["PLEX"].get("LIBRARY", DEFAULT_LIBRARY)
        DEFAULT_AD_COLLECTION = config["PLEX"].get(
            "AD_COLLECTION", DEFAULT_AD_COLLECTION)
        DEFAULT_COMMENTARY_COLLECTION = config["PLEX"].get(
            "COMMENTARY_COLLECTION", DEFAULT_COMMENTARY_COLLECTION)
        if config["PLEX"].get("TOKEN"):
            os.environ["PLEX_TOKEN"] = config["PLEX"].get("TOKEN")

# Keywords that indicate a commentary track (case-insensitive).
COMMENTARY_KEYWORDS = [
    "commentary",
    "director's commentary",
    "filmmaker commentary"]


# -- Data structures ---------------------------------------------------------

@dataclass
class ScanResult:
    ad_movies: list = field(default_factory=list)
    commentary_movies: list = field(default_factory=list)
    total_scanned: int = 0
    errors: int = 0


# -- Scanning ----------------------------------------------------------------

def has_ad_track(item) -> bool:
    """Return True if any audio stream on *item* is flagged visualImpaired."""
    for media in item.media:
        for part in media.parts:
            for stream in part.audioStreams():
                if getattr(stream, "visualImpaired", False):
                    return True
    return False


def has_commentary_track(item) -> bool:
    """Return True if any audio stream title contains a commentary keyword."""
    for media in item.media:
        for part in media.parts:
            for stream in part.audioStreams():
                title = (getattr(stream, "title", "") or "").lower()
                ext = (
                    getattr(
                        stream,
                        "extendedDisplayTitle",
                        "") or "").lower()
                combined = f"{title} {ext}"
                if any(kw in combined for kw in COMMENTARY_KEYWORDS):
                    return True
    return False


def scan_library(section, *, check_ad: bool,
                 check_commentary: bool) -> ScanResult:
    """Walk every item in *section* and classify AD / commentary tracks."""
    result = ScanResult()
    items = section.all()
    total = len(items)

    print(f"\nScanning {total} items in '{section.title}' ...\n")
    t0 = time.time()

    for i, item in enumerate(items, 1):
        pct = i / total * 100
        # Overwrite progress line in-place
        print(
            f"  [{i:>{len(str(total))}}/{total}] ({pct:5.1f}%)  {item.title[:60]:<60}", end="\r")

        try:
            # Reload to guarantee full stream metadata is available
            item.reload()

            if check_ad and has_ad_track(item):
                result.ad_movies.append(item)
            if check_commentary and has_commentary_track(item):
                result.commentary_movies.append(item)
        except Exception as exc:
            result.errors += 1
            print(f"\n  [WARNING]  Error scanning '{item.title}': {exc}")

        result.total_scanned += 1

    elapsed = time.time() - t0
    print(f"\n\nScan complete in {elapsed:.1f}s  "
          f"({result.total_scanned} scanned, {result.errors} errors)")
    return result


# -- Collection management --------------------------------------------------

def sync_collection(
    section,
    collection_name: str,
    wanted_items: list,
    *,
    dry_run: bool = False,
) -> dict:
    """
    Create or update a static collection so that it contains exactly
    *wanted_items* and nothing else.  Returns a summary dict.
    """
    wanted_keys = {item.ratingKey for item in wanted_items}

    # Try to fetch existing collection
    existing_collection = None
    try:
        existing_collection = section.collection(collection_name)
    except NotFound:
        pass

    summary = {"added": [], "removed": [], "unchanged": 0}

    if existing_collection is not None:
        # Determine current members
        current_items = existing_collection.items()
        current_keys = {item.ratingKey for item in current_items}

        # Items to add (in wanted but not in current)
        to_add = [
            item for item in wanted_items if item.ratingKey not in current_keys]
        # Items to remove (in current but not in wanted)
        to_remove = [
            item for item in current_items if item.ratingKey not in wanted_keys]

        summary["unchanged"] = len(current_keys & wanted_keys)

        if to_add:
            summary["added"] = [item.title for item in to_add]
            if not dry_run:
                existing_collection.addItems(to_add)

        if to_remove:
            summary["removed"] = [item.title for item in to_remove]
            if not dry_run:
                existing_collection.removeItems(to_remove)

    else:
        # Collection doesn't exist yet - create it by tagging the items
        summary["added"] = [item.title for item in wanted_items]
        if not dry_run:
            for item in wanted_items:
                item.addCollection(collection_name)

    # Delete the collection entirely if it would be empty
    if not wanted_items and existing_collection and not dry_run:
        existing_collection.delete()

    return summary


def print_summary(collection_name: str, summary: dict, dry_run: bool):
    prefix = "[DRY RUN] " if dry_run else ""
    print(f"\n{'-' * 60}")
    print(f"{prefix}Collection: {collection_name}")
    print(f"{'-' * 60}")
    if summary["added"]:
        print(f"  + Added ({len(summary['added'])}):")
        for t in sorted(summary["added"]):
            print(f"      * {t}")
    if summary["removed"]:
        print(f"  - Removed ({len(summary['removed'])}):")
        for t in sorted(summary["removed"]):
            print(f"      * {t}")
    if summary["unchanged"]:
        print(f"  [OK] Unchanged: {summary['unchanged']}")
    if not summary["added"] and not summary["removed"]:
        print("  (no changes)")


# -- CLI ---------------------------------------------------------------------

def parse_args():
    p = argparse.ArgumentParser(
        description="Maintain Plex static collections for AD & Commentary tracks."
    )
    p.add_argument("--url", default=os.environ.get("PLEX_URL", DEFAULT_PLEX_URL),
                   help="Plex server URL  [env: PLEX_URL]")
    p.add_argument("--token", default=os.environ.get("PLEX_TOKEN"),
                   help="Plex auth token   [env: PLEX_TOKEN]")
    p.add_argument("--library", default=DEFAULT_LIBRARY,
                   help=f"Library section name (default: {DEFAULT_LIBRARY})")
    p.add_argument("--ad-collection", default=DEFAULT_AD_COLLECTION,
                   help=f"AD collection name (default: {DEFAULT_AD_COLLECTION})")
    p.add_argument("--commentary-collection", default=DEFAULT_COMMENTARY_COLLECTION,
                   help=f"Commentary collection name (default: {DEFAULT_COMMENTARY_COLLECTION})")
    p.add_argument("--no-ad", action="store_true",
                   help="Skip the Audio Description scan")
    p.add_argument("--no-commentary", action="store_true",
                   help="Skip the Commentary scan")
    p.add_argument("--dry-run", action="store_true",
                   help="Preview changes without modifying Plex")
    return p.parse_args()


def main():
    args = parse_args()

    if not args.token:
        print("ERROR: Plex token is required.")
        print("       Set PLEX_TOKEN env var or pass --token <token>")
        print("       Find your token: https://support.plex.tv/articles/204059436/")
        sys.exit(1)

    if args.no_ad and args.no_commentary:
        print("ERROR: --no-ad and --no-commentary both set -- nothing to do.")
        sys.exit(1)

    check_ad = not args.no_ad
    check_commentary = not args.no_commentary

    # Connect
    print(f"Connecting to Plex at {args.url} ...")
    plex = PlexServer(args.url, args.token)
    print(f"  Server: {plex.friendlyName}  (v{plex.version})")

    section = plex.library.section(args.library)
    print(f"  Library: {section.title}  ({section.totalSize} items)")

    if args.dry_run:
        print("\n  *** DRY RUN -- no changes will be made ***")

    # Scan
    result = scan_library(
        section,
        check_ad=check_ad,
        check_commentary=check_commentary)

    if check_ad:
        print(
            f"\n  Found {len(result.ad_movies)} item(s) with Audio Description tracks")
    if check_commentary:
        print(
            f"  Found {len(result.commentary_movies)} item(s) with Commentary tracks")

    # Sync collections
    if check_ad:
        summary = sync_collection(
            section, args.ad_collection, result.ad_movies, dry_run=args.dry_run
        )
        print_summary(args.ad_collection, summary, args.dry_run)

    if check_commentary:
        summary = sync_collection(
            section, args.commentary_collection, result.commentary_movies, dry_run=args.dry_run
        )
        print_summary(args.commentary_collection, summary, args.dry_run)

    print()


if __name__ == "__main__":
    main()
