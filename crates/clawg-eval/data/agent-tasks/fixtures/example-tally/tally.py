import sys


def count_words(path):
    with open(path, encoding="utf-8") as f:
        return sum(len(line.split()) for line in f)


if __name__ == "__main__":
    for name in sys.argv[1:]:
        print(name, count_words(name))
