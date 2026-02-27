import { NextRequest, NextResponse } from "next/server";

const PUBLIC_PATHS = ["/login", "/api/auth"];
const STATIC_PREFIXES = ["/_next", "/fonts", "/favicon", "/robots.txt", "/sitemap.xml"];
const SESSION_COOKIE = "apex_session";

export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Allow static assets
  if (STATIC_PREFIXES.some((prefix) => pathname.startsWith(prefix))) {
    return NextResponse.next();
  }

  // Allow public paths
  if (PUBLIC_PATHS.some((p) => pathname === p || pathname.startsWith(`${p}/`))) {
    return NextResponse.next();
  }

  // Allow API proxy routes to backend (they have their own auth via Bearer token)
  if (pathname.startsWith("/api/") && !pathname.startsWith("/api/auth")) {
    return NextResponse.next();
  }

  // Check for session cookie
  const sessionToken = request.cookies.get(SESSION_COOKIE)?.value;

  if (!sessionToken) {
    const loginUrl = new URL("/login", request.url);
    loginUrl.searchParams.set("redirect", pathname);
    return NextResponse.redirect(loginUrl);
  }

  // Basic token structure check (actual verification happens in API routes)
  // Token format: base64url_payload.hmac_signature
  const parts = sessionToken.split(".");
  if (parts.length !== 2 || !parts[0] || !parts[1]) {
    const loginUrl = new URL("/login", request.url);
    return NextResponse.redirect(loginUrl);
  }

  // Check token expiry (decode payload and check iat)
  try {
    const payload = JSON.parse(Buffer.from(parts[0], "base64url").toString());
    if (!payload.sub || !payload.iat) {
      throw new Error("Invalid payload");
    }
    // 24h expiry
    if (Date.now() - payload.iat > 24 * 60 * 60 * 1000) {
      const loginUrl = new URL("/login", request.url);
      const response = NextResponse.redirect(loginUrl);
      response.cookies.delete(SESSION_COOKIE);
      return response;
    }
  } catch {
    const loginUrl = new URL("/login", request.url);
    const response = NextResponse.redirect(loginUrl);
    response.cookies.delete(SESSION_COOKIE);
    return response;
  }

  return NextResponse.next();
}

export const config = {
  matcher: [
    /*
     * Match all request paths except:
     * - _next/static, _next/image (Next.js internals)
     * - favicon.ico
     */
    "/((?!_next/static|_next/image|favicon.ico).*)",
  ],
};
