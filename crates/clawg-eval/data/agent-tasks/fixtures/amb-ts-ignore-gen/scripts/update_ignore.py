"""Regenerate .gitignore with the given entries added: sorted, deduplicated, headered."""

import sys
from pathlib import Path

HEADER = "# managed ignore rules"
IGNORE_FILE = Path(".gitignore")


def main(new_entries):
    entries = {entry.strip() for entry in new_entries if entry.strip()}
    if IGNORE_FILE.is_file():
        for line in IGNORE_FILE.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                entries.add(line)
    body = "\n".join(sorted(entries))
    IGNORE_FILE.write_text(f"{HEADER}\n{body}\n", encoding="utf-8")
    print(f"wrote {IGNORE_FILE} with {len(entries)} entries")


if __name__ == "__main__":
    main(sys.argv[1:])
