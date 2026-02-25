import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";
import { AppShell } from "@/components/app-shell";
import { Providers } from "./providers";

const apexSans = localFont({
  src: [
    { path: "../../public/fonts/InterVariable.woff2", style: "normal" },
    { path: "../../public/fonts/InterVariable-Italic.woff2", style: "italic" },
  ],
  variable: "--font-apex",
  display: "swap",
});

const apexMono = localFont({
  src: [
    { path: "../../public/fonts/JetBrainsMono-Variable.ttf", style: "normal" },
    { path: "../../public/fonts/JetBrainsMono-Italic-Variable.ttf", style: "italic" },
  ],
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  title: {
    default: "ApexIntel",
    template: "%s | ApexIntel",
  },
  description: "ApexIntel OSINT Intelligence Platform",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${apexSans.variable} ${apexMono.variable}`}>
        <a href="#main-content" className="skip-link">
          Skip to main content
        </a>
        <div className="fixed inset-0 border-[8px] border-rams-chassis pointer-events-none z-[100] hidden md:block" aria-hidden="true" />
        <Providers>
          <AppShell>{children}</AppShell>
        </Providers>
      </body>
    </html>
  );
}
