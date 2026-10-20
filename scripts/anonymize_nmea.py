#!/usr/bin/env python3
"""Shift every position in an NMEA capture to a public reference point.

Keeps timing, satellites and sentence structure; rewrites lat/lon in RMC, GGA,
GLL and GNS by a constant offset and recomputes checksums. Lines without a
valid checksum are dropped. Use before committing captures as test fixtures:
real receiver positions are personal data.

    python3 -I scripts/anonymize_nmea.py capture.nmea tests/fixtures/name.nmea [lat lon]

Default target: Biķernieki circuit, Rīga (56.9637 N, 24.2289 E).
"""
import sys

FIELDS = {"RMC": (3, 5), "GGA": (2, 4), "GLL": (1, 3), "GNS": (2, 4)}


def ddm(value, deg_digits):
    d = int(value)
    return f"{d:0{deg_digits}d}{(value - d) * 60:08.5f}"


def parse(text, deg_digits):
    return int(text[:deg_digits]) + float(text[deg_digits:]) / 60


def checksum(body):
    c = 0
    for ch in body.encode():
        c ^= ch
    return c


def main():
    src, dst = sys.argv[1], sys.argv[2]
    target = (float(sys.argv[3]), float(sys.argv[4])) if len(sys.argv) == 5 else (56.9637, 24.2289)
    ref, out, dropped = None, [], 0
    with open(src, encoding="ascii", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line.startswith("$") or "*" not in line:
                dropped += line != ""
                continue
            body, cs = line[1:].rsplit("*", 1)
            try:
                if int(cs[:2], 16) != checksum(body):
                    dropped += 1
                    continue
            except ValueError:
                dropped += 1
                continue
            f = body.split(",")
            idx = FIELDS.get(f[0][2:])
            if idx and f[idx[0]] and f[idx[1]]:
                lat = parse(f[idx[0]], 2) * (-1 if f[idx[0] + 1] == "S" else 1)
                lon = parse(f[idx[1]], 3) * (-1 if f[idx[1] + 1] == "W" else 1)
                if ref is None:
                    ref = (lat - target[0], lon - target[1])
                nlat, nlon = lat - ref[0], lon - ref[1]
                f[idx[0]], f[idx[0] + 1] = ddm(abs(nlat), 2), "N" if nlat >= 0 else "S"
                f[idx[1]], f[idx[1] + 1] = ddm(abs(nlon), 3), "E" if nlon >= 0 else "W"
            body = ",".join(f)
            out.append(f"${body}*{checksum(body):02X}")
    with open(dst, "w", encoding="ascii") as fh:
        fh.write("\n".join(out) + "\n")
    print(f"kept {len(out)} sentences, dropped {dropped}")


if __name__ == "__main__":
    main()
