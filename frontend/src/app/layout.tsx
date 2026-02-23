import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";
import { AppShell } from "@/components/app-shell";

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
  title: "ApexIntel",
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
        <AppShell>{children}</AppShell>
      </body>
    </html>
  );
}
