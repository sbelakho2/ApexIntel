#!/usr/bin/env python3
"""Download the official EU CPV 2008 code list and write it as CSV.

Source: the Publications Office of the EU distribution for the Common
Procurement Vocabulary (SKOS, version 2008):
https://publications.europa.eu/resource/distribution/cpv/rdf/skos_core/cpv-skos-core.rdf

The previous `cpv_codes_2008.csv` was a saved TED web page (CSS + HTML), not a
code list. This script regenerates the file from the authoritative RDF so the
dataset can be rebuilt and audited instead of trusted.

Usage:
    python3 download_cpv.py [output.csv]

Output columns: `code,description` (8-digit CPV codes, English prefLabels,
sorted ascending). The official CPV 2008 vocabulary contains 9,454 concepts.
"""

import csv
import os
import sys
import urllib.request
import xml.etree.ElementTree as ET

RDF_URL = (
    "https://publications.europa.eu/resource/distribution/cpv/rdf/skos_core/"
    "cpv-skos-core.rdf"
)
EXPECTED_CONCEPTS = 9454
RDF_NS = "http://www.w3.org/1999/02/22-rdf-syntax-ns#"
SKOS_NS = "http://www.w3.org/2004/02/skos/core#"
XML_LANG = "{http://www.w3.org/XML/1998/namespace}lang"


def download_rdf(path: str) -> None:
    request = urllib.request.Request(
        RDF_URL,
        headers={
            "User-Agent": "ApexIntel data-builder bot@research.local",
            # The Publications Office returns HTTP 406 for a bare
            # `application/rdf+xml` Accept header; the wildcard fallback keeps
            # the RDF response while satisfying its negotiation.
            "Accept": "application/rdf+xml;q=1, */*;q=0.1",
        },
    )
    with urllib.request.urlopen(request, timeout=540) as response, open(path, "wb") as fh:
        while chunk := response.read(1 << 20):
            fh.write(chunk)


def parse_concepts(rdf_path: str) -> list[tuple[str, str]]:
    rows: list[tuple[str, str]] = []
    for _, elem in ET.iterparse(rdf_path, events=("end",)):
        if elem.tag != f"{{{RDF_NS}}}Description":
            continue
        code = None
        label = None
        is_concept = False
        for child in elem:
            if child.tag == f"{{{RDF_NS}}}type":
                if child.get(f"{{{RDF_NS}}}resource") == f"{SKOS_NS}Concept":
                    is_concept = True
            elif child.tag == f"{{{SKOS_NS}}}notation":
                code = (child.text or "").strip()
            elif child.tag == f"{{{SKOS_NS}}}prefLabel":
                if child.get(XML_LANG) == "en":
                    label = (child.text or "").strip()
        if is_concept and code and label:
            rows.append((code, label))
        elem.clear()
    rows.sort(key=lambda row: row[0])
    return rows


def main() -> int:
    out_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "cpv_codes_2008.csv"
    )
    rdf_path = out_path + ".rdf.tmp"
    try:
        download_rdf(rdf_path)
        rows = parse_concepts(rdf_path)
    finally:
        if os.path.exists(rdf_path):
            os.remove(rdf_path)

    if len(rows) != EXPECTED_CONCEPTS:
        print(
            f"refusing to write: parsed {len(rows)} concepts, expected "
            f"{EXPECTED_CONCEPTS} — the distribution may have changed",
            file=sys.stderr,
        )
        return 1

    with open(out_path, "w", newline="", encoding="utf-8") as fh:
        writer = csv.writer(fh, quoting=csv.QUOTE_MINIMAL)
        writer.writerow(["code", "description"])
        writer.writerows(rows)

    print(f"wrote {len(rows)} CPV 2008 concepts to {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
