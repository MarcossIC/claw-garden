"""The command line of the shelf."""

import sys

from config import shelf_folder
from search import find


def main(argv):
    """Run the command line with the arguments in argv."""
    if len(argv) == 2 and argv[0] == "find":
        for name in find(shelf_folder(), argv[1]):
            print(name)
        return 0
    print("usage: shelf find WORD")
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
