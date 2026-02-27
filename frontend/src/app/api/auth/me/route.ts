import { NextResponse } from "next/server";
import { verifySessionToken, getSessionCookieName } from "@/lib/auth";
import { cookies } from "next/headers";

export async function GET() {
  const cookieStore = cookies();
  const token = cookieStore.get(getSessionCookieName())?.value;

  if (!token) {
    return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
  }

  const payload = verifySessionToken(token);
  if (!payload) {
    return NextResponse.json({ error: "Invalid session" }, { status: 401 });
  }

  return NextResponse.json({
    user: {
      username: payload.sub,
      role: "admin",
    },
  });
}
