import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Login | ApexIntel",
  description: "Authenticate to access ApexIntel Intelligence Platform",
};

export default function LoginLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Login page has its own standalone layout — no AppShell sidebar
  return <>{children}</>;
}
