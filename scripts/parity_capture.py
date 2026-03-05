import base64
import hashlib
import hmac
import json
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

BASE_URL = "https://starzerp.fi"
USERNAME = "aaron"
SESSION_SECRET = "94eaff21c290ca4ff353ed53470995cbbb2f2899b073bde2ada4d7d85ed97143"

ROUTES = {
    "/": "overview",
    "/warnings": "warnings",
    "/insights": "insights",
    "/memos": "memos",
    "/companies": "companies",
    "/persons": "persons",
    "/competitors": "competitors",
    "/security": "security",
    "/graph": "graph",
    "/recipes": "recipes",
    "/settings": "settings",
}


def build_session_token() -> str:
    now_ms = int(time.time() * 1000)
    payload_bytes = json.dumps({"sub": USERNAME, "iat": now_ms}, separators=(",", ":")).encode("utf-8")
    payload_b64 = base64.urlsafe_b64encode(payload_bytes).decode("ascii").rstrip("=")
    sig = hmac.new(SESSION_SECRET.encode("utf-8"), payload_bytes, hashlib.sha256).hexdigest()
    return f"{payload_b64}.{sig}"


def main() -> None:
    token = build_session_token()
    out_dir = Path("frontend/e2e/parity/current")
    out_dir.mkdir(parents=True, exist_ok=True)

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        context = browser.new_context(viewport={"width": 1536, "height": 864}, color_scheme="light")
        context.add_cookies(
            [
                {
                    "name": "apex_session",
                    "value": token,
                    "domain": "starzerp.fi",
                    "path": "/",
                    "httpOnly": True,
                    "secure": True,
                    "sameSite": "Lax",
                }
            ]
        )
        page = context.new_page()

        for route, name in ROUTES.items():
            url = f"{BASE_URL}{route}"
            page.goto(url, wait_until="networkidle", timeout=60000)
            page.wait_for_timeout(2200)
            page.mouse.move(1, 1)
            page.evaluate(
                """() => {
                    const active = document.activeElement;
                    if (active && typeof active.blur === 'function') {
                        active.blur();
                    }
                }"""
            )
            page.add_style_tag(content="*,:before,:after{animation:none!important;transition:none!important;}")

            target = out_dir / f"sense-rams-{name}-chromium-desktop-darwin.png"
            page.screenshot(path=str(target), full_page=True)
            print(f"{route}\t{target.name}\t{page.url}\t{page.title()}")

        context.close()
        browser.close()


if __name__ == "__main__":
    main()
