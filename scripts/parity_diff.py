from pathlib import Path
from PIL import Image, ImageChops, ImageStat

ROUTE_BY_NAME = {
    "overview": "/",
    "warnings": "/warnings",
    "insights": "/insights",
    "memos": "/memos",
    "companies": "/companies",
    "persons": "/persons",
    "competitors": "/competitors",
    "security": "/security",
    "graph": "/graph",
    "recipes": "/recipes",
    "settings": "/settings",
}

baseline_dir = Path("frontend/e2e/parity/baseline")
current_dir = Path("frontend/e2e/parity/current")
diff_dir = Path("frontend/e2e/parity/diff")
diff_dir.mkdir(parents=True, exist_ok=True)

rows = []
for baseline_path in sorted(baseline_dir.glob("*.png")):
    name = baseline_path.name
    current_path = current_dir / name
    page_key = name.replace("sense-rams-", "").replace("-chromium-desktop-darwin.png", "")
    route = ROUTE_BY_NAME.get(page_key, "")
    if not current_path.exists():
        rows.append((name, route, "missing-current", "", "", ""))
        continue

    with Image.open(baseline_path).convert("RGBA") as b_img, Image.open(current_path).convert("RGBA") as c_img:
        if b_img.size != c_img.size:
            min_w = min(b_img.width, c_img.width)
            min_h = min(b_img.height, c_img.height)
            b_cmp = b_img.crop((0, 0, min_w, min_h))
            c_cmp = c_img.crop((0, 0, min_w, min_h))
            size_note = f"baseline={b_img.size},current={c_img.size},cmp={(min_w, min_h)}"
        else:
            b_cmp = b_img
            c_cmp = c_img
            size_note = f"{b_img.size}"

        diff = ImageChops.difference(b_cmp, c_cmp)
        stat = ImageStat.Stat(diff)
        channels = len(stat.mean)
        mean_abs = sum(stat.mean) / channels
        mismatch_ratio = mean_abs / 255.0

        diff_l = diff.convert("L")
        hist = diff_l.histogram()
        changed_pixels = sum(hist[1:])
        total_pixels = b_cmp.width * b_cmp.height
        changed_ratio = changed_pixels / total_pixels if total_pixels else 0.0

        if changed_pixels > 0:
            boosted = diff.convert("RGB")
            boosted = ImageChops.multiply(boosted, Image.new("RGB", boosted.size, (6, 6, 6)))
            boosted.save(diff_dir / name)
        else:
            # Keep an explicit zero-diff artifact for consistency
            diff.convert("RGB").save(diff_dir / name)

        rows.append(
            (
                name,
                route,
                f"{mismatch_ratio:.6f}",
                f"{changed_ratio:.6f}",
                str(changed_pixels),
                size_note,
            )
        )

report_path = diff_dir / "report.tsv"
with report_path.open("w", encoding="utf-8") as f:
    f.write("file\troute\tmismatch_ratio\tchanged_px_ratio\tchanged_px\tsize\n")
    for row in rows:
        f.write("\t".join(row) + "\n")

for row in rows:
    print("\t".join(row))
print(f"REPORT\t{report_path}")
