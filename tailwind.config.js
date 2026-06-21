/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./crates/api/templates/**/*.html",
    "./crates/api/static/js/**/*.js",
    "./crates/frontend/src/**/*.rs",
  ],
  darkMode: "class",
  theme: {
    extend: {
      fontFamily: {
        display: ["var(--font-apex)", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["var(--font-mono)", "JetBrains Mono", "ui-monospace", "monospace"],
      },
      /* ── Rams Color Family ─────────────────────────────────────────── */
      colors: {
        rams: {
          chassis: "var(--rams-chassis)",
          module: "var(--rams-module)",
          panel: "var(--rams-panel)",
          line: "var(--rams-line)",
          "line-strong": "var(--rams-line-strong)",
          muted: "var(--rams-muted)",
          foreground: "var(--rams-foreground)",
          orange: "var(--rams-orange)",
          green: "var(--rams-green)",
          red: "var(--rams-red)",
          steel: "var(--rams-steel)",
        },
        /* ── Legacy shadcn-style names (mapped to Rams) ──────────────── */
        border: "rgb(var(--border) / <alpha-value>)",
        input: "rgb(var(--input) / <alpha-value>)",
        ring: "rgb(var(--ring) / <alpha-value>)",
        background: "rgb(var(--background) / <alpha-value>)",
        foreground: "rgb(var(--foreground) / <alpha-value>)",
        primary: {
          DEFAULT: "rgb(var(--primary) / <alpha-value>)",
          foreground: "rgb(var(--primary-foreground) / <alpha-value>)",
        },
        secondary: {
          DEFAULT: "rgb(var(--secondary) / <alpha-value>)",
          foreground: "rgb(var(--secondary-foreground) / <alpha-value>)",
        },
        destructive: {
          DEFAULT: "rgb(var(--destructive) / <alpha-value>)",
          foreground: "rgb(var(--destructive-foreground) / <alpha-value>)",
        },
        muted: {
          DEFAULT: "rgb(var(--muted) / <alpha-value>)",
          foreground: "rgb(var(--muted-foreground) / <alpha-value>)",
        },
        accent: {
          DEFAULT: "rgb(var(--accent) / <alpha-value>)",
          foreground: "rgb(var(--accent-foreground) / <alpha-value>)",
        },
        popover: {
          DEFAULT: "rgb(var(--popover) / <alpha-value>)",
          foreground: "rgb(var(--popover-foreground) / <alpha-value>)",
        },
        card: {
          DEFAULT: "rgb(var(--card) / <alpha-value>)",
          foreground: "rgb(var(--card-foreground) / <alpha-value>)",
        },
        success: "rgb(var(--success) / <alpha-value>)",
        warning: "rgb(var(--warning) / <alpha-value>)",
        danger: "rgb(var(--danger) / <alpha-value>)",
        info: "rgb(var(--info) / <alpha-value>)",
        error: "rgb(var(--error) / <alpha-value>)",
        surface: {
          50: "rgb(var(--surface-50) / <alpha-value>)",
          100: "rgb(var(--surface-100) / <alpha-value>)",
          200: "rgb(var(--surface-200) / <alpha-value>)",
          300: "rgb(var(--surface-300) / <alpha-value>)",
          400: "rgb(var(--surface-400) / <alpha-value>)",
          500: "rgb(var(--surface-500) / <alpha-value>)",
          600: "rgb(var(--surface-600) / <alpha-value>)",
          700: "rgb(var(--surface-700) / <alpha-value>)",
          800: "rgb(var(--surface-800) / <alpha-value>)",
          900: "rgb(var(--surface-900) / <alpha-value>)",
        },
        brand: {
          400: "rgb(var(--brand-400) / <alpha-value>)",
          500: "rgb(var(--brand-500) / <alpha-value>)",
          600: "rgb(var(--brand-600) / <alpha-value>)",
          700: "rgb(var(--brand-700) / <alpha-value>)",
        },
      },
      borderColor: {
        DEFAULT: "rgb(var(--border) / <alpha-value>)",
      },
      /* ── Rams Radii (micro only, no element exceeds 6px) ──────────── */
      borderRadius: {
        "rams-xs": "0",
        "rams-sm": "2px",
        "rams-md": "4px",
        "rams-lg": "6px",
        /* Legacy aliases */
        lg: "var(--radius-lg)",
        md: "var(--radius-md)",
        sm: "var(--radius-sm)",
      },
      /* ── Rams Spacing (4px grid) ───────────────────────────────────── */
      spacing: {
        "rams-1": "4px",
        "rams-2": "8px",
        "rams-3": "12px",
        "rams-4": "16px",
        "rams-6": "24px",
        "rams-8": "32px",
        "rams-12": "48px",
        "rams-16": "64px",
      },
      /* ── Rams Shadows (structural only, no floating) ──────────────── */
      boxShadow: {
        "rams-inset": "var(--rams-shadow-inset)",
        "rams-pressed": "var(--rams-shadow-pressed)",
        "rams-focus": "var(--rams-shadow-focus)",
        /* Legacy premium card shadows removed — anti-pattern */
      },
      /* ── Rams Transition Durations (≤200ms, utilitarian) ───────────── */
      transitionDuration: {
        "rams-instant": "50ms",
        "rams-fast": "120ms",
        "rams-normal": "200ms",
      },
      /* ── Rams Transition Timing ────────────────────────────────────── */
      transitionTimingFunction: {
        "rams-ease": "cubic-bezier(0.2, 0, 0, 1)",
      },
    },
  },
  plugins: [
    require("@tailwindcss/forms"),
    require("@tailwindcss/typography"),
    require("tailwindcss-animate"),
  ],
};
