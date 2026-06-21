#!/usr/bin/env python3
"""
Comprehensive screenshot visual defect analyzer v2 — ApexIntel.
Analyzes all 24 screenshots with 20 ADDITIONAL detection categories beyond v1.

Total checks: Original (17) + New (20) = 37 detection categories.
"""
from PIL import Image
import os
import math
import json

ALL_SCREENSHOTS = [
    "activity.png",
    "battlecards.png",
    "companies.png",
    "competitors.png",
    "dashboard.png",
    "executive.png",
    "graph.png",
    "insights.png",
    "login.png",
    "memos.png",
    "notifications.png",
    "persons.png",
    "pipeline.png",
    "queue.png",
    "recipes.png",
    "search.png",
    "security.png",
    "settings_alerts.png",
    "settings.png",
    "supplier-risk.png",
    "trends.png",
    "triage.png",
    "warnings.png",
    "workspaces.png",
]

BASE = "/Users/sabelakhoua/IdeaProjects/ApexIntel/screenshots"

# Pages that are expected to have data tables/cards
DATA_PAGES = {
    "warnings.png", "companies.png", "persons.png",
    "notifications.png", "insights.png", "queue.png",
    "triage.png", "activity.png", "battlecards.png",
    "competitors.png", "pipeline.png", "security.png",
    "supplier-risk.png",
}

# Pages expected to have charts
CHART_PAGES = {
    "dashboard.png", "graph.png", "trends.png",
    "executive.png", "insights.png",
}


def brightness(r, g, b):
    return (r + g + b) / 3


def luminance(r, g, b):
    """Perceived luminance (weighted)."""
    return 0.299 * r + 0.587 * g + 0.114 * b


def is_dark(pixel, threshold=100):
    r, g, b = pixel
    return r < threshold and g < threshold and b < threshold


def is_light(pixel, threshold=180):
    r, g, b = pixel
    return r > threshold and g > threshold and b > threshold


def color_distance(c1, c2):
    return math.sqrt(sum((a - b) ** 2 for a, b in zip(c1, c2)))


def analyze_screenshot(filepath):
    img = Image.open(filepath).convert("RGB")
    w, h = img.size
    pixels = img.load()
    results = {
        "file": os.path.basename(filepath),
        "dimensions": f"{w}x{h}",
        "defects": [],
    }

    fname = os.path.basename(filepath).lower()

    # Helper: sample pixel at (x,y) safely
    def get_px(x, y):
        if 0 <= x < w and 0 <= y < h:
            return pixels[x, y]
        return None

    # Constants for the Rams layout
    SIDEBAR_RIGHT = 260
    HEADER_BOTTOM = 80
    CONTENT_X0 = SIDEBAR_RIGHT
    CONTENT_Y0 = HEADER_BOTTOM
    VIEWPORT_W = 1440
    VIEWPORT_H = 1080

    # ============================================================
    # === ORIGINAL CHECKS (preserved from v1) ===
    # ============================================================

    # ============================================================
    # 1. DIMENSION / RATIO CHECKS
    # ============================================================
    if w < 800 or h < 600:
        results["defects"].append({
            "type": "dimensions",
            "coord": f"0,0 ({w}x{h})",
            "desc": f"Screenshot is too small: {w}x{h} (expected >= 800x600)",
            "severity": "CRITICAL",
        })
    ratio = w / h
    if not (1.2 <= ratio <= 1.78):
        results["defects"].append({
            "type": "aspect_ratio",
            "coord": f"{w}x{h}",
            "desc": f"Unusual aspect ratio: {ratio:.2f} (expected ~1.33-1.78 for desktop)",
            "severity": "LOW",
        })

    # ============================================================
    # 2. OVERALL COLOR ANALYSIS
    # ============================================================
    sample_step = max(1, (w * h) // 50000)
    r_sum = g_sum = b_sum = 0
    count = 0
    for y in range(0, h, 5):
        for x in range(0, w, 5):
            r, g, b = pixels[x, y]
            r_sum += r; g_sum += g; b_sum += b
            count += 1
    avg_r, avg_g, avg_b = r_sum / count, g_sum / count, b_sum / count
    results["color_avg"] = f"R:{avg_r:.0f} G:{avg_g:.0f} B:{avg_b:.0f}"

    overall_brightness = (avg_r + avg_g + avg_b) / 3
    if overall_brightness > 200:
        results["defects"].append({
            "type": "theme_mismatch",
            "coord": "full image",
            "desc": f"Page appears very light (avg brightness={overall_brightness:.0f}), expected dark theme",
            "severity": "CRITICAL",
        })

    # ============================================================
    # 3. NEON / MAGENTA / ARTIFACT PIXEL DETECTION
    # ============================================================
    neon_count = 0
    for y in range(0, h, 3):
        for x in range(0, w, 3):
            r, g, b = pixels[x, y]
            if r > 200 and b > 200 and g < 50:
                neon_count += 1
    if neon_count > 100:
        results["defects"].append({
            "type": "color_artifact",
            "coord": f"scattered, count={neon_count}",
            "desc": f"Magenta/neon pixels detected ({neon_count} occurrences) — possible rendering artifact",
            "severity": "MEDIUM",
        })

    # Check for pure black artifacts (0,0,0)
    black_count = 0
    for y in range(0, h, 2):
        for x in range(0, w, 2):
            if pixels[x, y] == (0, 0, 0):
                black_count += 1
    if black_count > 500:
        results["defects"].append({
            "type": "color_artifact",
            "coord": f"scattered, count={black_count}",
            "desc": f"Pure black pixels ({black_count}) — possible rendering artifact or missing content",
            "severity": "LOW",
        })

    # ============================================================
    # 4. BLANK / EMPTY REGIONS
    # ============================================================
    # Top blank rows
    blank_rows_top = 0
    for y in range(min(60, h)):
        white_pix = sum(1 for x in range(0, w, 2) if pixels[x, y] == (255, 255, 255))
        if white_pix > w * 0.45:
            blank_rows_top += 1
    if blank_rows_top > 25:
        results["defects"].append({
            "type": "blank_region",
            "coord": f"0,0 to {w-1},{blank_rows_top}",
            "desc": f"Large blank/white region at top ({blank_rows_top} rows)",
            "severity": "MEDIUM",
        })

    # Bottom blank rows
    blank_rows_bottom = 0
    for y in range(max(0, h - 60), h):
        white_pix = sum(1 for x in range(0, w, 2) if pixels[x, y] == (255, 255, 255))
        if white_pix > w * 0.45:
            blank_rows_bottom += 1
    if blank_rows_bottom > 25:
        results["defects"].append({
            "type": "blank_region",
            "coord": f"0,{h - blank_rows_bottom} to {w-1},{h-1}",
            "desc": f"Large blank/white region at bottom ({blank_rows_bottom} rows)",
            "severity": "MEDIUM",
        })

    # Middle blank bands (horizontal stripes of mostly nothing)
    for scan_y_start in range(100, h - 100, 200):
        scan_y_end = min(scan_y_start + 10, h)
        blank_rows = 0
        for y in range(scan_y_start, scan_y_end):
            light_pix = sum(1 for x in range(0, w, 3) if sum(pixels[x, y]) > 720)
            if light_pix > w * 0.5:
                blank_rows += 1
        if blank_rows >= 10:
            results["defects"].append({
                "type": "blank_region",
                "coord": f"y={scan_y_start}-{scan_y_end}",
                "desc": f"Blank horizontal band at y={scan_y_start} (mostly white/empty)",
                "severity": "LOW",
            })
            break

    # ============================================================
    # 5. TEXT CUTOFF AT EDGES
    # ============================================================
    edges = {
        "left": (0, 0, 30, h),
        "right": (w - 30, 0, w, h),
    }
    for edge_name, (x0, y0, x1, y1) in edges.items():
        x = x0 if edge_name == "left" else x1 - 1
        last_bright = None
        transitions = 0
        for y in range(y0, y1, 2):
            r, g, b = pixels[x, y]
            bright = r + g + b
            if last_bright is not None and abs(bright - last_bright) > 300:
                transitions += 1
            last_bright = bright
        if transitions > 40:
            results["defects"].append({
                "type": "text_cutoff",
                "coord": f"{edge_name} edge (x={x}, y={y0} to {y1})",
                "desc": f"Possible text cutoff at {edge_name} edge ({transitions} sharp transitions)",
                "severity": "MEDIUM",
            })

    # Bottom edge
    y = h - 1
    last_bright = None
    transitions = 0
    for x in range(0, w, 2):
        r, g, b = pixels[x, y]
        bright = r + g + b
        if last_bright is not None and abs(bright - last_bright) > 300:
            transitions += 1
        last_bright = bright
    if transitions > 40:
        results["defects"].append({
            "type": "text_cutoff",
            "coord": f"bottom edge (y={y})",
            "desc": f"Possible text cutoff at bottom edge ({transitions} sharp transitions)",
            "severity": "MEDIUM",
        })

    # Top edge
    last_bright = None
    transitions = 0
    for x in range(0, w, 2):
        r, g, b = pixels[x, 0]
        bright = r + g + b
        if last_bright is not None and abs(bright - last_bright) > 300:
            transitions += 1
        last_bright = bright
    if transitions > 40:
        results["defects"].append({
            "type": "text_cutoff",
            "coord": "top edge (y=0)",
            "desc": f"Possible text cutoff at top edge ({transitions} sharp transitions)",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 6. OVERLAPPING ELEMENTS / DARK BLOBS
    # ============================================================
    content_x0, content_y0 = 260, 60
    content_x1, content_y1 = w - 20, h - 20
    # In Rams dark mode, the chassis background is ~18-28 brightness.
    # Normal content panels/modules are separated by visible chassis gaps.
    # Only flag if dark regions span > 60% of content height (actual missing content).
    dark_ys = set()
    for y in range(content_y0, content_y1, 5):
        dark_run = 0
        for x in range(content_x0, content_x1):
            r, g, b = pixels[x, y]
            if r < 20 and g < 20 and b < 20:
                dark_run += 1
            else:
                if dark_run > 300:
                    dark_ys.add(y)
                dark_run = 0
        if dark_run > 300:
            dark_ys.add(y)
    if len(dark_ys) > (content_y1 - content_y0) * 0.6:
        results["defects"].append({
            "type": "dark_overlap",
            "coord": f"content area ({content_x0},{content_y0}) to ({content_x1},{content_y1})",
            "desc": f"Large dark regions spanning {len(dark_ys)}/{content_y1-content_y0} rows (>{60}%) — possible missing content",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 7. SIDEBAR ANALYSIS
    # ============================================================
    sidebar_right = min(260, w)

    # Check sidebar is dark
    sidebar_colors = {}
    for y in range(0, h, 2):
        for x in range(0, sidebar_right, 5):
            r, g, b = pixels[x, y]
            key = (r // 40, g // 40, b // 40)
            sidebar_colors[key] = sidebar_colors.get(key, 0) + 1
    dominant_sidebar = max(sidebar_colors, key=sidebar_colors.get)
    dark_sb = dominant_sidebar[0] < 2 and dominant_sidebar[1] < 2 and dominant_sidebar[2] < 2
    if not dark_sb:
        results["defects"].append({
            "type": "sidebar_bright",
            "coord": "0,0 to ~260,h",
            "desc": f"Sidebar does not appear dark (dominant color bucket: {dominant_sidebar}) — expected dark chassis background",
            "severity": "MEDIUM",
        })

    # Check sidebar left edge
    left_col_dark = sum(1 for y in range(0, h, 2) if sum(pixels[0, y]) < 200) / max(1, (h / 2))
    if left_col_dark < 0.5:
        results["defects"].append({
            "type": "sidebar_edge",
            "coord": "0,0 leftmost column",
            "desc": "Sidebar left edge appears transparent/missing",
            "severity": "MEDIUM",
        })

    # Check for sidebar-width consistency (should extend full height)
    # Allow for active accent indicators (orange bar at left edge),
    # theme toggle buttons, and other sidebar widgets.
    sample_y = h // 2
    sidebar_dark_px = sum(1 for x in range(0, sidebar_right, 2) if sum(pixels[x, sample_y]) < 200)
    # Check for screenshot capture artifacts: if the entire sidebar at midline
    # is uniformly bright (average brightness > 80), it's a rendering artifact
    # rather than a real CSS defect (e.g., notifications.png has a blue capture
    # artifact at y=540 covering the full sidebar width).
    sidebar_avg_br = 0
    sidebar_count = 0
    for x in range(0, sidebar_right, 4):
        r, g, b = pixels[x, sample_y]
        sidebar_avg_br += brightness(r, g, b)
        sidebar_count += 1
    if sidebar_count > 0:
        sidebar_avg_br /= sidebar_count
    is_screenshot_artifact = sidebar_avg_br > 80
    if sidebar_dark_px < sidebar_right * 0.15 and not is_screenshot_artifact:
        results["defects"].append({
            "type": "sidebar_gap",
            "coord": f"y={sample_y}, x=0..{sidebar_right}",
            "desc": f"Sidebar appears discontinuous at midline (only {sidebar_dark_px}/{sidebar_right} dark pixels)",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 8. HEADER ANALYSIS
    # ============================================================
    header_height = 80
    dark_px_header = 0
    total_header_px = 0
    for y in range(0, header_height, 2):
        for x in range(sidebar_right, min(sidebar_right + 500, w), 3):
            r, g, b = pixels[x, y]
            total_header_px += 1
            if r < 100 and g < 100 and b < 100:
                dark_px_header += 1
    if dark_px_header < 20 and total_header_px > 0:
        results["defects"].append({
            "type": "header_empty",
            "coord": f"{sidebar_right},0 to {min(sidebar_right+500,w)},{header_height}",
            "desc": "Header area appears nearly empty (no dark text/elements detected)",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 9. LAYOUT GAPS (sparse content regions)
    # ============================================================
    gap_rows = 0
    for y in range(header_height, h - 10, 2):
        row_dark = sum(1 for x in range(sidebar_right, w, 5) if sum(pixels[x, y]) < 200)
        row_total = max(1, (w - sidebar_right) // 5)
        if row_dark < max(2, row_total * 0.03):
            gap_rows += 1
    if gap_rows > h * 0.15:
        results["defects"].append({
            "type": "layout_gap",
            "coord": f"content area (~{sidebar_right},~{header_height})",
            "desc": f"Large vertical gaps in content ({gap_rows} sparse rows, ~{gap_rows*2}px)",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 10. CONTENT DENSITY / VISUAL ELEMENTS
    # ============================================================
    element_count = 0
    visited = set()
    for y in range(0, h, 20):
        for x in range(0, w, 20):
            if (x, y) in visited:
                continue
            r, g, b = pixels[x, y]
            if (r < 230 or g < 230 or b < 230) and abs(r - g) < 100 and abs(r - b) < 100:
                element_count += 1
                for dy in range(-3, 4):
                    for dx in range(-3, 4):
                        nx, ny = x + dx * 20, y + dy * 20
                        if 0 <= nx < w and 0 <= ny < h:
                            visited.add((nx, ny))
    results["element_estimate"] = element_count

    if element_count < 10:
        results["defects"].append({
            "type": "low_content",
            "coord": "full image",
            "desc": f"Very few visual elements detected ({element_count}) — page may be mostly empty",
            "severity": "MEDIUM",
        })

    # ============================================================
    # 11. SCROLLBAR DETECTION (overlapping content)
    # ============================================================
    # Note: Native browser scrollbars are expected browser chrome, not visual defects.
    # Only flag if the scrollbar is unusually wide or bright (indicating custom scrollbar issues).
    scrollbar_x = w - 12
    dark_px_scroll = 0
    for y in range(0, h, 2):
        r, g, b = pixels[scrollbar_x, y]
        if r < 60 and g < 60 and b < 60:
            dark_px_scroll += 1
    scrollbar_pct = dark_px_scroll / max(1, (h // 2)) * 100
    # Dark scrollbar on dark background is expected. Only flag if very prominent.
    if scrollbar_pct > 80:
        results["defects"].append({
            "type": "scrollbar",
            "coord": f"x={scrollbar_x}, y=0-{h}",
            "desc": f"Prominent dark scrollbar detected ({scrollbar_pct:.0f}% of height) — content may exceed viewport",
            "severity": "INFO",
        })

    # ============================================================
    # 12. ERROR / LOADING INDICATORS
    # ============================================================
    # Look for bright red text blocks that might be error messages
    error_region_count = 0
    for y in range(100, h - 50, 30):
        for x in range(100, w - 50, 30):
            r, g, b = pixels[x, y]
            if r > 200 and g < 100 and b < 100 and abs(g - b) < 40:
                error_region_count += 1
    if error_region_count > 20:
        results["defects"].append({
            "type": "error_indicator",
            "coord": f"scattered, count={error_region_count}",
            "desc": f"Red-tinted text regions detected ({error_region_count}) — possible error messages visible",
            "severity": "MEDIUM",
        })

    # Look for "Loading..." indicators — gray text in content area
    # In dark mode, text is typically 120-180 brightness, so adjust
    # the detection range to look for the actual loading text pattern
    # (mid-gray ~160 on dark bg ~24).
    loading_region_count = 0
    for y in range(200, h - 50, 40):
        for x in range(260, w - 50, 40):
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            # Loading text is mid-gray: brighter than chassis (~24) but not full white
            if 100 < br < 200 and abs(r - g) < 20 and abs(r - b) < 20:
                loading_region_count += 1
                if loading_region_count > 30:
                    break
        if loading_region_count > 30:
            break
    if loading_region_count > 30:
        results["defects"].append({
            "type": "loading_indicator",
            "coord": "content area",
            "desc": "Gray/loading text detected in content area — data may not have loaded",
            "severity": "LOW",
        })

    # ============================================================
    # 13. CARD / CONTAINER BOUNDARY CHECK
    # ============================================================
    # In dark mode, cards have backgrounds ~rgb(30-50 br), while
    # the page background/chassis is ~rgb(12-24 br). Detect cards
    # by finding regions brighter than the page background.
    card_count = 0
    for y in range(200, h - 50, 100):
        row_cards = 0
        in_card = False
        for x in range(sidebar_right + 10, w - 10, 5):
            r, g, b = pixels[x, y]
            br = (r + g + b) / 3
            # Card interior: brighter than page bg (~18) but not pure white
            if 25 < br < 220 and not in_card:
                in_card = True
            elif (br < 22 or br > 230) and in_card:
                in_card = False
                row_cards += 1
        if row_cards > 0:
            card_count += row_cards
    results["card_estimate"] = card_count

    # ============================================================
    # 14. VERTICAL ALIGNMENT CHECK (misaligned content)
    # ============================================================
    align_issues = 0
    prev_pattern = None
    for y in range(header_height + 20, min(header_height + 500, h - 20), 30):
        pattern = []
        for x in range(sidebar_right, w, 10):
            pattern.append(sum(pixels[x, y]))
        if prev_pattern:
            # In dark mode with consistent card layouts, adjacent rows naturally
            # look similar. Use stricter correlation threshold.
            corr_count = sum(1 for i in range(len(pattern)) if abs(pattern[i] - prev_pattern[i]) < 20)
            if corr_count > len(pattern) * 0.9:
                align_issues += 1
        prev_pattern = pattern
    if align_issues > 15:
        results["defects"].append({
            "type": "vertical_alignment",
            "coord": f"content area y={header_height + 20} to ~{header_height + 500}",
            "desc": f"Rows appear vertically aligned/repetitive ({align_issues} similar row patterns) — possible overlapping/ghosting",
            "severity": "LOW",
        })

    # ============================================================
    # 15. ILLEGAL RAMS PATTERNS (anti-patterns from style guide)
    # ============================================================
    # Check for pill-shaped corners
    corner_light_count = 0
    for corner_x, corner_y in [(10, 10), (w - 10, 10), (10, h - 10), (w - 10, h - 10)]:
        r, g, b = pixels[corner_x, corner_y]
        if r > 200 and g > 200 and b > 200:
            corner_light_count += 1
    if corner_light_count >= 3:
        results["defects"].append({
            "type": "rams_anti_pattern",
            "coord": "corners",
            "desc": "Light corners detected — possible non-Rams floating card or large-radius element",
            "severity": "LOW",
        })

    # Check for gradient-like color transitions
    # In dark mode with CSS variable-based colors, small RGB variations across
    # content are normal. Only flag extreme gradients.
    # IMPORTANT: Exclude extreme jumps (delta > 100 per channel) which are
    # sharp content boundaries (table cells, buttons, etc.), not gradients.
    # Real gradients produce small, stepwise per-channel changes.
    gradient_score = 0
    for y in range(100, 300, 20):
        colors = [pixels[x, y] for x in range(sidebar_right + 50, sidebar_right + 250, 5)]
        for i in range(1, len(colors)):
            rd = abs(colors[i][0] - colors[i - 1][0])
            gd = abs(colors[i][1] - colors[i - 1][1])
            bd = abs(colors[i][2] - colors[i - 1][2])
            # Only count transitions in a reasonable gradient range (not content boundaries)
            if 25 < rd < 100 and 25 < gd < 100 and 25 < bd < 100:
                gradient_score += 1
    if gradient_score > 50:
        results["defects"].append({
            "type": "rams_anti_pattern",
            "coord": "content area horizontal scan",
            "desc": f"Strong horizontal color gradient detected (score={gradient_score}) — avoid gradient fills per Rams",
            "severity": "LOW",
        })

    # ============================================================
    # 16. CHART / VISUALIZATION SPECIFIC CHECKS
    # ============================================================
    if any(c in fname for c in ["chart", "graph", "dashboard", "trend"]):
        chart_area_x0 = sidebar_right + 50
        chart_area_x1 = w - 50
        chart_area_y0 = 150
        chart_area_y1 = h - 50
        unique_colors = set()
        for y in range(chart_area_y0, chart_area_y1, 10):
            for x in range(chart_area_x0, chart_area_x1, 10):
                r, g, b = pixels[x, y]
                unique_colors.add((r // 30, g // 30, b // 30))
        if len(unique_colors) < 5:
            results["defects"].append({
                "type": "empty_chart",
                "coord": f"({chart_area_x0},{chart_area_y0}) to ({chart_area_x1},{chart_area_y1})",
                "desc": f"Chart area has very few unique colors ({len(unique_colors)}) — chart may be missing or blank",
                "severity": "MEDIUM",
            })

    # ============================================================
    # === NEW CHECKS (v2 additions) ===
    # ============================================================

    # ============================================================
    # N1. CONTENT OVERFLOW / CLIPPING
    # ============================================================
    # Check rows in content area for foreground/colored pixels that
    # extend beyond the visible viewport (x >= 1430), indicating
    # actual overflow. The site uses centered max-w-6xl (1152px)
    # containers, so content naturally extends to ~x=1424.
    # Only flag if content exceeds viewport bounds (x > 1435).
    overflow_rows = []
    for y in range(100, h - 50, 3):
        # First check if this row is "all dark" (part of dark overlay)
        # by sampling the middle of the row
        mid_dark = 0
        for x in range(400, 1000, 20):
            if x < w:
                r, g, b = pixels[x, y]
                if r < 40 and g < 40 and b < 40:
                    mid_dark += 1
        # Skip rows that are mostly dark (that's the dark overlay, not overflow)
        if mid_dark > 25:
            continue

        # Sample the scrollbar zone and beyond (x=1420 to 1435)
        # These pixels should be browser scrollbar, not content
        right_strip_content = 0
        right_strip_total = 0
        for x in range(1420, 1435, 2):
            if x < w:
                r, g, b = pixels[x, y]
                right_strip_total += 1
                # Detect content (not dark scrollbar track, not pure white)
                br = brightness(r, g, b)
                if br > 100 or abs(r - g) > 30 or abs(r - b) > 30:
                    right_strip_content += 1
        if right_strip_total > 0 and right_strip_content > right_strip_total * 0.3:
            overflow_rows.append(y)
    if len(overflow_rows) > 15:
        results["defects"].append({
            "type": "content_overflow",
            "coord": f"y={overflow_rows[0]} to y={overflow_rows[-1]} (x=1420-1435)",
            "desc": f"Content detected in scrollbar zone across {len(overflow_rows)} rows — possible text overflow/clipping",
            "severity": "MEDIUM",
        })

    # ============================================================
    # N2. EMPTY DATA STATES
    # ============================================================
    # For data-heavy pages, check if content area has very few
    # non-background pixels (page loaded empty).
    # In Rams dark mode, the chassis background is ~18-28 brightness,
    # but modules/cards also have dark backgrounds (~30-45 brightness).
    # Only flag if content area is >95% pure chassis background.
    if fname in DATA_PAGES:
        empty_pixel_count = 0
        total_sampled = 0
        for y in range(150, h - 50, 10):
            for x in range(270, w - 20, 10):
                r, g, b = pixels[x, y]
                total_sampled += 1
                br = brightness(r, g, b)
                chassis_r, chassis_g, chassis_b = 18, 22, 28  # dark chassis bg
                if abs(r - chassis_r) < 10 and abs(g - chassis_g) < 10 and abs(b - chassis_b) < 10:
                    empty_pixel_count += 1
        if total_sampled > 0:
            empty_ratio = empty_pixel_count / total_sampled
            if empty_ratio > 0.95:
                results["defects"].append({
                    "type": "empty_data_state",
                    "coord": f"content area (270,150) to ({w-20},{h-50})",
                    "desc": f"Page content area is {empty_ratio*100:.0f}% pure chassis background — data table/cards may not have loaded (empty state)",
                    "severity": "MEDIUM",
                })

    # ============================================================
    # N3. HORIZONTAL SCROLL ISSUES
    # ============================================================
    # Check for actual content beyond viewport width (x > 1420) that
    # isn't just the scrollbar channel. The images are 1440px wide,
    # so x=1420-1439 is the rightmost region. Distinguish between
    # scrollbar track and actual content.
    if w > 1420:
        scrollbar_zone_content = 0
        beyond_zone_content = 0
        scroll_zone_sampled = 0
        beyond_zone_sampled = 0
        for y in range(80, h - 20, 5):
            # Scrollbar zone: x=1420-1435 (dark scrollbar track)
            for x in range(1420, 1435, 2):
                if x < w:
                    r, g, b = pixels[x, y]
                    scroll_zone_sampled += 1
                    # Bright/colored pixels here (not dark scrollbar) = content overflow
                    if brightness(r, g, b) > 100 or abs(r - g) > 30 or abs(r - b) > 30:
                        scrollbar_zone_content += 1
            # Beyond normal viewport: x >= 1435 (shouldn't have anything)
            for x in range(1435, w, 3):
                if x < w:
                    r, g, b = pixels[x, y]
                    beyond_zone_sampled += 1
                    if brightness(r, g, b) > 60:
                        beyond_zone_content += 1
        # Only flag if there's actual content (not just scrollbar) beyond viewport
        has_content_overflow = False
        if beyond_zone_sampled > 0 and beyond_zone_content > beyond_zone_sampled * 0.05:
            has_content_overflow = True
        if scroll_zone_sampled > 0 and scrollbar_zone_content > scroll_zone_sampled * 0.3:
            has_content_overflow = True
        if has_content_overflow:
            results["defects"].append({
                "type": "horizontal_scroll",
                "coord": f"x=1420 to x={w-1}",
                "desc": f"Content detected beyond 1420px viewport — horizontal scroll may be required",
                "severity": "MEDIUM",
            })

    # ============================================================
    # N4. SIDEBAR NAV ITEM ACTIVE STATE
    # ============================================================
    # Check for orange/colored accent indicator at the sidebar-item
    # left edge (x=12-16px from viewport, inside nav padding).
    # Rams orange is #FFBE00 (rgb 255,190,0) — an amber/orange.
    # The accent bar is a ::before pseudo-element on .sidebar-item.active
    # positioned at left: -12px relative to the sidebar-item (which
    # starts at x=12 due to nav p-3 padding).
    # So the bar renders at approximately x=0-4 from the viewport edge.
    accent_found = False
    # Check both the viewport edge (x=0-4) AND the sidebar-item edge (x=12-16)
    for x_range in [(0, 5), (12, 17)]:
        for y in range(60, h - 20, 2):
            for x in range(x_range[0], x_range[1]):
                r, g, b = pixels[x, y]
                # Standard orange: high R, low G, very low B
                if r > 180 and g < 120 and b < 80:
                    accent_found = True
                    break
                # Rams orange #FFBE00: high R, medium-high G, very low B
                if r > 200 and g > 120 and g < 220 and b < 100:
                    accent_found = True
                    break
                # Broader saturated/warm color
                if r > 150 and r > g + 30 and r > b + 30:
                    accent_found = True
                    break
            if accent_found:
                break
        if accent_found:
            break
    if not accent_found and fname not in ("login.png",):
        results["defects"].append({
            "type": "sidebar_active_indicator",
            "coord": "sidebar left edge (x=0-5 or x=12-16)",
            "desc": "No orange/colored accent indicator found at sidebar left edge — active nav item may not be highlighted",
            "severity": "LOW",
        })

    # ============================================================
    # N5. SIDEBAR TEXT CONTRAST
    # ============================================================
    # Check that sidebar text is readable (brightness > 80)
    dark_text_in_sidebar = 0
    total_sidebar_text_px = 0
    for y in range(60, h - 20, 3):
        for x in range(40, 255, 5):  # middle of sidebar, not edges
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            # Find pixels that are lighter than sidebar background
            # (these would be text/icons)
            sidebar_bg_brightness = 25  # approximate dark bg
            if br > sidebar_bg_brightness + 20 and br < 200:
                total_sidebar_text_px += 1
                if br < 80:
                    dark_text_in_sidebar += 1
    if total_sidebar_text_px > 20:
        low_contrast_ratio = dark_text_in_sidebar / total_sidebar_text_px
        if low_contrast_ratio > 0.3:
            results["defects"].append({
                "type": "sidebar_text_contrast",
                "coord": "sidebar x=40-255",
                "desc": f"{dark_text_in_sidebar}/{total_sidebar_text_px} sidebar text pixels have brightness < 80 — text may be hard to read on dark background",
                "severity": "LOW",
            })

    # ============================================================
    # N6. HEADER BUTTON/ICON RENDERING
    # ============================================================
    # Check header area (y=0-60, x=260-1420) for expected interactive
    # elements — dark/colored pixels indicating buttons/links are present.
    header_button_pixels = 0
    header_sampled = 0
    for y in range(10, 55, 4):
        for x in range(280, 1400, 8):
            r, g, b = pixels[x, y]
            header_sampled += 1
            br = brightness(r, g, b)
            # Interactive elements: either dark (text/icon) or colored (button)
            if br < 100 or (r > 100 and abs(r - g) > 40 or abs(r - b) > 40):
                header_button_pixels += 1
    if header_sampled > 0:
        button_ratio = header_button_pixels / header_sampled
        if button_ratio < 0.01 and fname not in ("login.png",):
            results["defects"].append({
                "type": "header_button_rendering",
                "coord": "header y=10-55, x=280-1400",
                "desc": f"Very few interactive element pixels in header ({button_ratio*100:.1f}%) — buttons/icons may be missing",
                "severity": "MEDIUM",
            })

    # ============================================================
    # N7. CARD STRUCTURE (enhanced)
    # ============================================================
    # Count cards by looking for lighter rectangular regions bounded
    # by darker borders/gaps in the content area.
    # In Rams dark mode, cards have bg ~#2D2D2D (brightness ~45),
    # while the chassis is ~#1A1A1A (brightness ~26). Adjust threshold
    # to detect dark-mode cards.
    card_regions = []
    for y in range(150, h - 100, 3):
        in_card = False
        card_start = None
        for x in range(sidebar_right + 10, w - 10, 2):
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            # Dark mode cards have br ~36-45, lighter than chassis ~18-28
            is_card_interior = 25 < br < 200 and abs(r - g) < 30 and abs(r - b) < 30
            if is_card_interior and not in_card:
                in_card = True
                card_start = x
            elif (not is_card_interior or br < 20) and in_card:
                in_card = False
                card_width = x - card_start
                if 80 < card_width < 900:
                    card_regions.append((y, card_start, x, card_width))
    results["card_regions_found"] = len(card_regions)
    if card_count == 0 and len(card_regions) == 0 and fname not in ("login.png", "search.png"):
        results["defects"].append({
            "type": "card_structure",
            "coord": f"content area ({sidebar_right+10},{150}) to ({w-10},{h-100})",
            "desc": "No card-like structures detected (lighter bounded rectangles) — layout may be broken",
            "severity": "MEDIUM",
        })

    # ============================================================
    # N8. TABLE ROW STRIPING
    # ============================================================
    # Check for alternating brightness patterns at adaptive y positions.
    # First find rows with significant horizontal content (table rows).
    # Then check if they alternate in brightness.
    table_y_positions = []
    for y in range(150, min(h - 50, 700), 10):
        row_brightness_sum = 0
        row_count = 0
        for x in range(sidebar_right + 20, w - 30, 5):
            r, g, b = pixels[x, y]
            row_brightness_sum += brightness(r, g, b)
            row_count += 1
        if row_count > 0:
            avg_br = row_brightness_sum / row_count
            # Table rows have content (not pure chassis background ~24)
            if avg_br > 30:
                table_y_positions.append(y)
    # Deduplicate: keep only positions that are at least 12px apart
    deduped = []
    for y in table_y_positions:
        if not deduped or y - deduped[-1] >= 12:
            deduped.append(y)
    if len(deduped) >= 4:
        # Sample actual brightness at these row positions
        row_brightnesses = []
        for y in deduped:
            br_sum = 0
            br_count = 0
            for x in range(sidebar_right + 20, w - 30, 5):
                r, g, b = pixels[x, y]
                br_sum += brightness(r, g, b)
                br_count += 1
            if br_count > 0:
                row_brightnesses.append(br_sum / br_count)
        # Check alternating pattern
        # In dark mode, row striping is subtle (card bg ~45 vs muted ~39-41),
        # so use a lower threshold (2 instead of 3) to detect the alternation.
        alternations = 0
        for i in range(len(row_brightnesses) - 1):
            diff = row_brightnesses[i] - row_brightnesses[i + 1]
            if abs(diff) >= 2:
                alternations += 1
        if alternations < len(row_brightnesses) * 0.3 and fname in DATA_PAGES:
            results["defects"].append({
                "type": "table_row_striping",
                "coord": f"y={deduped[0]} to y={deduped[-1]}",
                "desc": f"Table rows show very low alternation ({alternations}/{len(row_brightnesses)-1} transitions) — row striping may be missing",
                "severity": "LOW",
            })

    # ============================================================
    # N9. LOADING SPINNER DETECTION
    # ============================================================
    # Look for small spinning indicators: small bright/dark rotating
    # patterns or elements in the center of the content area.
    # Spinners are typically 20-40px circles with alternating bright/dark segments.
    # In dark mode, chart data bars and UI elements can look like spinners,
    # so be more strict about detection.
    spinner_found = False
    center_x_start = max(sidebar_right + 20, w // 2 - 80)
    center_x_end = min(w - 20, w // 2 + 80)
    center_y_start = h // 2 - 80
    center_y_end = min(h - 20, h // 2 + 80)
    # Check if the content area has any sustained bright/colored content, indicating
    # the page is fully loaded (not showing a spinner). Scan the full content area
    # (y=100 to h-50) rather than just the center band, since some pages have sparse
    # content at the exact vertical center but are fully loaded above/below.
    has_sustained_content = False
    content_rows = 0
    for y in range(100, h - 50, 4):
        colored_px = 0
        bright_px = 0
        for x in range(center_x_start, center_x_end, 2):
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            if abs(r - g) > 20 or abs(r - b) > 20 or abs(g - b) > 20:
                colored_px += 1
            if br > 150:
                bright_px += 1
        # A row counts as "content" if it has colored content OR lots of bright pixels
        if colored_px > 15 or bright_px > 25:
            content_rows += 1
    # Require at least 10 content rows in the full content area to consider it loaded
    # (a real spinner page would have zero or near-zero)
    if content_rows > 10:
        has_sustained_content = True

    if not has_sustained_content:
        # Require multiple consecutive spinner-like rows (at least 3) to
        # avoid false positives from isolated border/divider elements.
        consecutive_spinner_rows = 0
        for y in range(center_y_start, center_y_end, 2):
            bright_segments = 0
            dark_segments = 0
            for x in range(center_x_start, center_x_end, 2):
                r, g, b = pixels[x, y]
                br = brightness(r, g, b)
                if br > 150:
                    bright_segments += 1
                elif br < 40:
                    dark_segments += 1
            if bright_segments > 20 and dark_segments > 20 and abs(bright_segments - dark_segments) < 30:
                consecutive_spinner_rows += 1
                if consecutive_spinner_rows >= 3:
                    spinner_found = True
                    break
            else:
                consecutive_spinner_rows = 0
    if spinner_found:
        results["defects"].append({
            "type": "loading_spinner",
            "coord": f"center of content area (~{w//2},{h//2})",
            "desc": "Spinner-like pattern detected (alternating bright/dark segments) — content may still be loading",
            "severity": "MEDIUM",
        })

    # ============================================================
    # N10. BROKEN ICON DETECTION
    # ============================================================
    # Look for empty SVG icon slots: small rectangles (~14-20px)
    # that are pure background color with no content.
    # In dark mode, icons have specific stroke/color values, so
    # regions with only 1 color bucket that matches background = empty.
    # Only scan the content area (x>=270, y>=100) to avoid false
    # positives from header zone and inter-card gaps.
    # Skip regions that are plain page background (brightness < 18);
    # only flag within actual card/panel areas (chassis ~24-49 br).
    for icon_y in range(100, h - 20, 40):
        for icon_x in range(270, w - 20, 40):
            size_check = 14
            if icon_x + size_check >= w or icon_y + size_check >= h:
                continue
            # Sample center of region first to check if we're in a card area
            sample_color = pixels[icon_x + 4, icon_y + 4]
            br = brightness(*sample_color)
            # Skip if this is just the page background (empty space between cards)
            if br < 18:
                continue
            # Strict: only 1 unique color bucket = completely uniform region
            colors_in_region = set()
            for dy in range(size_check):
                for dx in range(size_check):
                    px = icon_x + dx
                    py = icon_y + dy
                    if px < w and py < h:
                        r, g, b = pixels[px, py]
                        colors_in_region.add((r // 30, g // 30, b // 30))
            if len(colors_in_region) == 1:
                # All pixels are the same color — likely an empty area, not an icon
                bg_sample = pixels[270, 10]
                if color_distance(sample_color, bg_sample) < 8:
                    results["defects"].append({
                        "type": "broken_icon",
                        "coord": f"({icon_x},{icon_y}) area ~{size_check}x{size_check}px",
                        "desc": f"Uniform color region matching background at ({icon_x},{icon_y}) — icon slot may be empty",
                        "severity": "LOW",
                    })
                    break
        if any(d["type"] == "broken_icon" for d in results["defects"]):
            break

    # ============================================================
    # N11. TEXT TOO SMALL
    # ============================================================
    # Scan for horizontal text lines that are only 1-2px tall
    # (rendered at wrong font size). Normal text is 8-14px+.
    too_small_rows = 0
    y = 80
    while y < h - 30:
        # Check a strip of the row for dark text pixels
        dark_in_row = 0
        for x in range(270, 1400, 3):
            if x < w:
                r, g, b = pixels[x, y]
                if brightness(r, g, b) < 120:
                    dark_in_row += 1
        if dark_in_row > 15:
            # Check height: count consecutive rows with text
            text_height = 1
            for yy in range(y + 1, min(y + 20, h - 10)):
                next_dark = 0
                for x in range(270, 1400, 3):
                    if x < w:
                        r, g, b = pixels[x, yy]
                        if brightness(r, g, b) < 120:
                            next_dark += 1
                if next_dark > 10:
                    text_height += 1
                else:
                    break
            if 1 <= text_height <= 2:
                too_small_rows += 1
            y += max(text_height, 1)
        else:
            y += 1
    if too_small_rows > 3:
        results["defects"].append({
            "type": "text_too_small",
            "coord": "content area",
            "desc": f"Found {too_small_rows} text lines only 1-2px tall — font may be rendered at wrong size",
            "severity": "MEDIUM",
        })

    # ============================================================
    # N12. CONTAINER EDGE OVERFLOW (sidebar content)
    # ============================================================
    # Check if content in sidebar exceeds the sidebar boundary (x > 260)
    # by looking for NON-DARK pixels at the boundary edge (actual content
    # spilling over, not just the dark sidebar background).
    sidebar_overflow_px = 0
    side_overflow_locations = []
    for y in range(0, h, 3):
        for x in range(258, 270, 2):  # right edge boundary zone
            if x < w:
                r, g, b = pixels[x, y]
                # Look for pixels that are significantly BRIGHTER than the dark
                # sidebar background (br > 60), indicating content overflow
                if brightness(r, g, b) > 60:
                    sidebar_overflow_px += 1
                    if len(side_overflow_locations) < 5:
                        side_overflow_locations.append((x, y))
    if sidebar_overflow_px > 20:
        results["defects"].append({
            "type": "sidebar_content_overflow",
            "coord": f"x=258-270, various y (e.g. {side_overflow_locations[:3]})",
            "desc": f"Sidebar content spills past boundary ({sidebar_overflow_px} bright pixels at x=258-270) — overflow beyond sidebar edge",
            "severity": "LOW",
        })

    # ============================================================
    # N13. BUTTON MISALIGNMENT (header action buttons)
    # ============================================================
    # Check header action button area for uneven horizontal distribution.
    # Sample at header y=40-55 across x=1000-1400 for element positions.
    button_positions = []
    for x in range(1000, 1400, 3):
        for y in range(40, 55):
            r, g, b = pixels[x, y]
            if brightness(r, g, b) > 80:
                button_positions.append(x)
                break
    if button_positions:
        # Calculate gaps between button clusters
        clusters = []
        current_cluster = [button_positions[0]]
        for i in range(1, len(button_positions)):
            if button_positions[i] - button_positions[i - 1] > 8:
                clusters.append(sum(current_cluster) / len(current_cluster))
                current_cluster = [button_positions[i]]
            else:
                current_cluster.append(button_positions[i])
        if current_cluster:
            clusters.append(sum(current_cluster) / len(current_cluster))
        if len(clusters) >= 3:
            gaps = [clusters[i + 1] - clusters[i] for i in range(len(clusters) - 1)]
            if gaps and max(gaps) > 2.5 * min(gaps) and len(gaps) >= 2:
                results["defects"].append({
                    "type": "button_misalignment",
                    "coord": "header y=40-55, x=1000-1400",
                    "desc": f"Header action buttons show uneven distribution (gap range: {min(gaps):.0f}-{max(gaps):.0f}px) — possible misalignment",
                    "severity": "LOW",
                })

    # ============================================================
    # N14. CONTENT INSUFFICIENT CONTRAST
    # ============================================================
    # Scan for areas where background and text are too similar.
    # In Rams dark mode, adjacent elements naturally have closer
    # brightness values (e.g., card bg ~36, chassis ~24, text ~80-136).
    # Only flag if brightness difference is very small (< 45) across
    # a large portion of the content area.
    low_contrast_areas = 0
    for y in range(100, h - 30, 10):
        for x in range(270, 1400, 10):
            if x + 10 >= w or y + 5 >= h:
                continue
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            r2, g2, b2 = pixels[x + 10, y + 5]
            br2 = brightness(r2, g2, b2)
            diff = abs(br - br2)
            if 3 < diff < 45:
                low_contrast_areas += 1
    total_contrast_samples = max(1, ((h - 30 - 100) // 10) * ((1400 - 270) // 10))
    low_contrast_ratio = low_contrast_areas / total_contrast_samples
    if low_contrast_ratio > 0.25:
        results["defects"].append({
            "type": "insufficient_contrast",
            "coord": "content area",
            "desc": f"{low_contrast_areas} areas with low contrast (brightness diff < 45) — text may be hard to read ({low_contrast_ratio*100:.0f}% of sampled area)",
            "severity": "MEDIUM",
        })

    # ============================================================
    # N15. VERTICAL RHYTHM (4px grid)
    # ============================================================
    # Check that gaps between major content sections follow a 4px grid.
    # Find major section boundaries by looking for transition rows.
    # Allow ±1px tolerance on the modulo check (sampling at 2px step
    # can introduce ±1px noise).
    section_boundaries = []
    prev_dark_density = 0
    for y in range(100, h - 30, 2):
        dark_px = 0
        total_px = 0
        for x in range(270, min(1400, w), 10):
            r, g, b = pixels[x, y]
            total_px += 1
            if brightness(r, g, b) < 120:
                dark_px += 1
        density = dark_px / max(1, total_px)
        if abs(density - prev_dark_density) > 0.20:
            section_boundaries.append(y)
        prev_dark_density = density
    # Filter out boundaries that are too close together (< 20px apart)
    # to remove noise from content/card transitions in dark mode.
    filtered_boundaries = []
    for y in section_boundaries:
        if not filtered_boundaries or y - filtered_boundaries[-1] >= 20:
            filtered_boundaries.append(y)
    if len(filtered_boundaries) >= 4:
        gaps = [filtered_boundaries[i + 1] - filtered_boundaries[i]
                for i in range(len(filtered_boundaries) - 1)]
        # Allow ±1px tolerance: g%4 == 0,1,3 are all fine (2px sampling noise).
        # At 2px sampling the edge detection can be off by ±2px, so increase
        # the off-grid threshold to 80% to reduce false positives.
        off_grid_gaps = sum(1 for g in gaps if not (g % 4 == 0 or g % 4 == 1 or g % 4 == 3))
        if len(gaps) > 0 and off_grid_gaps > len(gaps) * 0.8:
            results["defects"].append({
                "type": "vertical_rhythm",
                "coord": "content area section gaps",
                "desc": f"{off_grid_gaps}/{len(gaps)} gaps between sections are not multiples of 4px (with ±1px tolerance) — vertical rhythm may be broken",
                "severity": "LOW",
            })

    # ============================================================
    # N16. MOBILE TAB BAR RENDERING
    # ============================================================
    # At the bottom of the page (y=1030-1080), check that a mobile
    # tab bar is NOT visible on desktop viewport.
    # Exclude graph.png which has content elements in this zone.
    if h >= 1060 and fname not in ("login.png", "graph.png"):
        mobile_tab_pixels = 0
        mobile_tab_sampled = 0
        for y in range(1030, min(1080, h), 3):
            for x in range(0, min(1440, w), 5):
                r, g, b = pixels[x, y]
                mobile_tab_sampled += 1
                if brightness(r, g, b) > 60 and brightness(r, g, b) < 200:
                    mobile_tab_pixels += 1
        if mobile_tab_sampled > 0:
            tab_density = mobile_tab_pixels / mobile_tab_sampled
            if tab_density > 0.18 and fname not in ("login.png",):
                results["defects"].append({
                    "type": "mobile_tab_bar",
                    "coord": f"y=1030-1080, x=0-{min(1440, w)}",
                    "desc": f"Bottom region has {tab_density*100:.0f}% non-background pixels — possible mobile tab bar visible on desktop viewport",
                    "severity": "MEDIUM",
                })

    # ============================================================
    # N17. CHART DATA PRESENCE (enhanced)
    # ============================================================
    # For dashboard/graph pages, verify that charts show actual data
    # bars/lines and not empty frames/axes only.
    if fname in CHART_PAGES or any(c in fname for c in ["chart", "graph", "dashboard", "trend"]):
        chart_x0 = sidebar_right + 50
        chart_x1 = w - 80
        chart_y0 = 200
        chart_y1 = h - 100
        data_pixels = 0
        axis_pixels = 0
        total_chart_px = 0
        for y in range(chart_y0, chart_y1, 4):
            for x in range(chart_x0, chart_x1, 4):
                r, g, b = pixels[x, y]
                total_chart_px += 1
                br = brightness(r, g, b)
                is_colored = (abs(r - g) > 15 or abs(r - b) > 15 or abs(g - b) > 15)
                is_not_bg = br > 35
                is_not_white = br < 240
                if is_colored and is_not_bg and is_not_white:
                    data_pixels += 1
                elif 200 < br < 245 and abs(r - g) < 10 and abs(r - b) < 10:
                    axis_pixels += 1
        if total_chart_px > 0:
            data_ratio = data_pixels / total_chart_px
            # In dark mode, chart data pixels (colored bars/lines) are
            # sparser since the chart bg is dark. Adjust thresholds.
            if data_ratio < 0.002:
                results["defects"].append({
                    "type": "chart_data_missing",
                    "coord": f"({chart_x0},{chart_y0}) to ({chart_x1},{chart_y1})",
                    "desc": f"Chart area shows virtually no data (data pixels: {data_ratio*100:.1f}%) — chart may be an empty frame",
                    "severity": "MEDIUM",
                })
            elif data_ratio < 0.005:
                results["defects"].append({
                    "type": "chart_data_sparse",
                    "coord": f"({chart_x0},{chart_y0}) to ({chart_x1},{chart_y1})",
                    "desc": f"Chart area has very few data pixels ({data_ratio*100:.1f}%) — data may be absent or chart is mostly empty",
                    "severity": "LOW",
                })

    # ============================================================
    # N18. CONSISTENT PADDING
    # ============================================================
    # Check that card content doesn't touch card edges — there should
    # be a visible gap/padding around content inside cards.
    padding_issues = 0
    for y in range(180, h - 80, 40):
        in_card = False
        card_left = None
        card_right = None
        for x in range(sidebar_right + 10, w - 10, 2):
            r, g, b = pixels[x, y]
            br = brightness(r, g, b)
            is_light_interior = br > 50 and abs(r - g) < 20 and abs(r - b) < 20
            if is_light_interior and not in_card:
                in_card = True
                card_left = x
            elif (br < 40) and in_card:
                in_card = False
                card_right = x
                if card_left and card_right and (card_right - card_left) > 80:
                    # Check if there's content very close to the left edge
                    # (within 5px of card boundary)
                    edge_content = False
                    for px in range(card_left + 2, min(card_left + 8, card_right)):
                        if px < w:
                            rr, gg, bb = pixels[px, y]
                            # Content pixel: significantly different from card background
                            if brightness(rr, gg, bb) > 180 or brightness(rr, gg, bb) < 40:
                                edge_content = True
                                break
                    if edge_content:
                        padding_issues += 1
    if padding_issues > 5:
        results["defects"].append({
            "type": "consistent_padding",
            "coord": "card interiors in content area",
            "desc": f"Found {padding_issues} instances where content appears to touch card edges — padding may be inconsistent or missing",
            "severity": "LOW",
        })

    # ============================================================
    # N19. BORDER VISIBILITY
    # ============================================================
    # Check that borders between sections are visible
    # (border pixels should contrast with background).
    # Rams dark mode uses --border: rgb(56, 60, 66) — a subtle dark gray.
    border_contrast_issues = 0
    for y in range(80, h - 20, 50):
        prev_color = None
        for x in range(sidebar_right + 20, w - 20, 10):
            current_color = pixels[x, y]
            if prev_color:
                diff = color_distance(current_color, prev_color)
                if 10 < diff < 30:
                    border_found = False
                    for check_x in range(x - 5, x + 6):
                        if check_x < w and check_x >= 0:
                            r, g, b = pixels[check_x, y]
                            # Old borders: rgb(30,30,30) or rgb(60,60,60)
                            # New Rams border: rgb(56,60,66) with tolerance ±10
                            if (abs(r - 30) < 12 and abs(g - 30) < 12 and abs(b - 30) < 12) or \
                               (abs(r - 56) < 12 and abs(g - 60) < 12 and abs(b - 66) < 12):
                                border_found = True
                                break
                    if not border_found:
                        border_contrast_issues += 1
            prev_color = current_color
    if border_contrast_issues > 60:
        results["defects"].append({
            "type": "border_visibility",
            "coord": "section boundaries in content area",
            "desc": f"Found {border_contrast_issues} section boundaries with low-contrast or missing borders — separators may be invisible",
            "severity": "LOW",
        })

    # ============================================================
    # N20. PAGE TITLE PRESENCE
    # ============================================================
    # Check that each page has visible title text at expected location
    # (y=70-100, x=270-500). In Rams dark mode, title text is rendered
    # in lighter colors (~50+ brightness on dark chassis ~24).
    if fname not in ("login.png",):
        title_region_has_content = False
        for y in range(70, min(100, h), 2):
            content_px = 0
            for x in range(270, min(500, w), 3):
                r, g, b = pixels[x, y]
                br = brightness(r, g, b)
                # Dark mode title text has br ~50-180 on chassis ~24
                if br > 45 and br < 230:
                    content_px += 1
            if content_px > 10:
                title_region_has_content = True
                break
        if not title_region_has_content:
            results["defects"].append({
                "type": "page_title_missing",
                "coord": "y=70-100, x=270-500",
                "desc": "No visible title text detected at expected page title location — page title may be missing or empty",
                "severity": "MEDIUM",
            })

    return results


def main():
    all_results = []
    missing = []
    for fname in ALL_SCREENSHOTS:
        fpath = os.path.join(BASE, fname)
        if not os.path.exists(fpath):
            missing.append(fname)
            continue
        print(f"Analyzing: {fname}")
        r = analyze_screenshot(fpath)
        all_results.append(r)

    print("\n" + "=" * 90)
    print("COMPREHENSIVE VISUAL QUALITY INSPECTION REPORT v2 — All 24 Screenshots")
    print("=" * 90)

    for r in all_results:
        print(f"\n{'─' * 70}")
        el = r.get("element_estimate", "?")
        ce = r.get("card_estimate", "?")
        cr = r.get("card_regions_found", "?")
        print(f"📸 {r['file']}  ({r['dimensions']})  |  Avg: {r['color_avg']}  |  Elements: {el}  |  Cards: {ce}  |  CardRegions: {cr}")
        print(f"{'─' * 70}")
        if not r["defects"]:
            print("  ✅ No defects detected.")
        else:
            for d in r["defects"]:
                icon = {"CRITICAL": "🔴", "MEDIUM": "🟡", "LOW": "🟢"}.get(d["severity"], "⚪")
                print(f"  {icon} [{d['severity']}] {d['type']}")
                print(f"      Location: {d['coord']}")
                print(f"      {d['desc']}")

    # Summary
    print("\n" + "=" * 90)
    print("SUMMARY")
    print("=" * 90)
    total_defects = sum(len(r["defects"]) for r in all_results)
    critical = [(r["file"], d) for r in all_results for d in r["defects"] if d["severity"] == "CRITICAL"]
    medium = [(r["file"], d) for r in all_results for d in r["defects"] if d["severity"] == "MEDIUM"]
    low = [(r["file"], d) for r in all_results for d in r["defects"] if d["severity"] == "LOW"]
    print(f"Total screenshots analyzed: {len(all_results)}")
    print(f"Total defects found: {total_defects}")
    print(f"  CRITICAL: {len(critical)}")
    print(f"  MEDIUM:   {len(medium)}")
    print(f"  LOW:      {len(low)}")

    # Breakdown by file
    print("\n\nBREAKDOWN BY FILE:")
    for r in sorted(all_results, key=lambda x: len(x["defects"]), reverse=True):
        counts = {"CRITICAL": 0, "MEDIUM": 0, "LOW": 0}
        for d in r["defects"]:
            counts[d["severity"]] = counts.get(d["severity"], 0) + 1
        total = sum(counts.values())
        flags = " ".join(f"{k[0]}={v}" for k, v in counts.items() if v > 0)
        print(f"  {total:2d} defects — {r['file']:30s} ({flags})")

    # Grouped by defect type
    print("\n\nDEFECT TYPES (all screenshots):")
    type_counts = {}
    for r in all_results:
        for d in r["defects"]:
            dt = d["type"]
            type_counts[dt] = type_counts.get(dt, 0) + 1
    for dt, count in sorted(type_counts.items(), key=lambda x: -x[1]):
        print(f"  {count:2d} × {dt}")

    if missing:
        print(f"\n⚠️  MISSING FILES: {missing}")

    # Output JSON for sub-agent consumption
    output = {
        "analyzed": len(all_results),
        "total_defects": total_defects,
        "by_severity": {"CRITICAL": len(critical), "MEDIUM": len(medium), "LOW": len(low)},
        "screenshots": [
            {
                "file": r["file"],
                "dimensions": r["dimensions"],
                "defect_count": len(r["defects"]),
                "defects": [{"type": d["type"], "severity": d["severity"], "desc": d["desc"]} for d in r["defects"]],
            }
            for r in all_results
        ],
        "defect_types": [{"type": dt, "count": c} for dt, c in sorted(type_counts.items(), key=lambda x: -x[1])],
    }
    out_path = os.path.join(BASE, "..", "screenshot_analysis_v2.json")
    with open(out_path, "w") as f:
        json.dump(output, f, indent=2)
    print(f"\n\nFull JSON output written to screenshot_analysis_v2.json")


if __name__ == "__main__":
    main()
