/**
 * Server-side API proxy for authenticated backend requests.
 * 
 * This route proxies requests to the Rust backend while adding the API key
 * header server-side, preventing the API key from being exposed to clients.
 * 
 * Security:
 * - API key (APEX_API_KEY) is stored server-side only
 * - Client authenticates via session cookie (verified by middleware)
 * - This proxy only forwards requests from authenticated sessions
 */

import { NextRequest, NextResponse } from "next/server";
import { verifySessionToken } from "@/lib/auth";

const BACKEND_URL = process.env.APEX_API_BASE_URL || "http://127.0.0.1:8080";
const API_KEY = process.env.APEX_API_KEY || "";

// Validate required configuration at startup
if (!API_KEY && process.env.NODE_ENV === "production") {
  console.error("FATAL: APEX_API_KEY environment variable is required in production");
}

async function proxyRequest(
  request: NextRequest,
  pathSegments: string[],
  method: string
): Promise<NextResponse> {
  // Verify session before proxying
  const sessionCookie = request.cookies.get("apex_session")?.value;
  if (!sessionCookie) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const session = verifySessionToken(sessionCookie);
  if (!session) {
    return NextResponse.json({ error: "Invalid or expired session" }, { status: 401 });
  }

  // Build the backend URL
  const path = `/api/${pathSegments.join("/")}`;
  const url = new URL(path, BACKEND_URL);
  
  // Copy query parameters
  request.nextUrl.searchParams.forEach((value, key) => {
    url.searchParams.set(key, value);
  });

  // Build headers for backend request
  const headers: HeadersInit = {
    "Content-Type": "application/json",
  };
  
  // Add API key authorization
  if (API_KEY) {
    headers["Authorization"] = `Bearer ${API_KEY}`;
  }

  // Forward origin for CORS checks
  const origin = request.headers.get("origin");
  if (origin) {
    headers["Origin"] = origin;
  }

  // Build request options
  const fetchOptions: RequestInit = {
    method,
    headers,
  };

  // Add body for POST/PUT/PATCH requests
  if (["POST", "PUT", "PATCH"].includes(method)) {
    try {
      const body = await request.text();
      if (body) {
        fetchOptions.body = body;
      }
    } catch {
      // No body is fine for some requests
    }
  }

  try {
    const response = await fetch(url.toString(), fetchOptions);
    
    // Get response body
    const responseText = await response.text();
    
    // Return proxied response with same status
    return new NextResponse(responseText, {
      status: response.status,
      headers: {
        "Content-Type": response.headers.get("Content-Type") || "application/json",
      },
    });
  } catch (error) {
    console.error("Proxy error:", error);
    return NextResponse.json(
      { error: "Backend service unavailable" },
      { status: 502 }
    );
  }
}

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const { path } = await params;
  return proxyRequest(request, path, "GET");
}

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const { path } = await params;
  return proxyRequest(request, path, "POST");
}

export async function PUT(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const { path } = await params;
  return proxyRequest(request, path, "PUT");
}

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const { path } = await params;
  return proxyRequest(request, path, "DELETE");
}

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const { path } = await params;
  return proxyRequest(request, path, "PATCH");
}
