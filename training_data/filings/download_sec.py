#!/usr/bin/env python3
"""Download SEC EDGAR submission data for EMS companies."""
import urllib.request
import json
import time
import os

DEST = os.path.join(os.path.dirname(os.path.abspath(__file__)), "10k")
os.makedirs(DEST, exist_ok=True)

HEADERS = {
    "User-Agent": "ApexIntel Research bot@research.local",
    "Accept": "application/json",
}

EMS_TICKERS = {
    "JBL": ("Jabil Inc", "0000898293"),
    "FLEX": ("Flex Ltd", "0000866374"),
    "CLS": ("Celestica Inc", "0001061219"),
    "BHE": ("Benchmark Electronics", "0000864883"),
    "PLXS": ("Plexus Corp", "0000785786"),
    "SANM": ("Sanmina Corp", "0000897723"),
    "TTMI": ("TTM Technologies", "0001137774"),
}

total = 0
for ticker, (name, cik) in EMS_TICKERS.items():
    print(f"\n{name} ({ticker}, CIK: {cik})...")
    url = f"https://data.sec.gov/submissions/CIK{cik}.json"
    req = urllib.request.Request(url, headers=HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read())

        dest_file = os.path.join(DEST, f"{ticker}_submissions.json")
        with open(dest_file, "w") as f:
            json.dump(data, f, indent=2)
        sz = os.path.getsize(dest_file)
        total += sz

        forms = data.get("filings", {}).get("recent", {}).get("form", [])
        tenk_count = sum(1 for f in forms if f == "10-K")
        print(f"  Saved {ticker}_submissions.json ({sz:,}b) - {tenk_count} 10-K filings found")

        recent = data.get("filings", {}).get("recent", {})
        accessions = recent.get("accessionNumber", [])
        form_types = recent.get("form", [])
        dates = recent.get("filingDate", [])
        prim_docs = recent.get("primaryDocument", [])

        tenk_filings = []
        for j in range(len(form_types)):
            if form_types[j] == "10-K" and j < len(accessions):
                acc = accessions[j].replace("-", "")
                doc = prim_docs[j] if j < len(prim_docs) else ""
                cik_clean = cik.lstrip("0")
                filing_url = f"https://www.sec.gov/Archives/edgar/data/{cik_clean}/{acc}/{doc}"
                tenk_filings.append({
                    "date": dates[j] if j < len(dates) else "",
                    "accession": accessions[j],
                    "url": filing_url,
                    "document": doc,
                })

        if tenk_filings:
            idx_file = os.path.join(DEST, f"{ticker}_10k_index.json")
            with open(idx_file, "w") as f:
                json.dump(
                    {"company": name, "ticker": ticker, "cik": cik, "filings": tenk_filings},
                    f,
                    indent=2,
                )
            print(f"  Index: {len(tenk_filings)} 10-K filing URLs saved")

    except Exception as e:
        print(f"  Error: {e}")
    time.sleep(0.2)

print(f"\nTotal: {total:,}b")
