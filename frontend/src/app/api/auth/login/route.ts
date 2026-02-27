import { NextResponse } from "next/server";
import { validateCredentials, createSessionToken, getSessionCookieName } from "@/lib/auth";

export async function POST(request: Request) {
  try {
    const body = await request.json();
    const { username, password } = body;

    if (!username || !password) {
      return NextResponse.json(
        { error: "Username and password are required" },
        { status: 400 }
      );
    }

    if (!validateCredentials(username, password)) {
      // Intentional delay to slow brute force
      await new Promise((r) => setTimeout(r, 800 + Math.random() * 400));
      return NextResponse.json(
        { error: "Invalid credentials" },
        { status: 401 }
      );
    }

    const token = createSessionToken(username);
    const response = NextResponse.json({ ok: true, user: { username, role: "admin" } });

    response.cookies.set(getSessionCookieName(), token, {
      httpOnly: true,
      secure: process.env.NODE_ENV === "production",
      sameSite: "lax",
      path: "/",
      maxAge: 24 * 60 * 60, // 24 hours
    });

    return response;
  } catch {
    return NextResponse.json({ error: "Invalid request" }, { status: 400 });
  }
}
