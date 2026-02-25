"use client";

import { useState } from "react";
import {
  PageHeader, SurfaceCard, StatCard,
  Database, Shield, Globe, Radio, Check, Clock
} from "@/components/ui";
import { SETTINGS_SECTIONS } from "@/lib/mock-data";

/**
 * Settings configuration panel with toggles, text, and select controls.
 */
export default function SettingsPage() {
  /* Local state mirrors mock data for demo interactivity */
  const [sections, setSections] = useState(SETTINGS_SECTIONS);
  const [isSaving, setIsSaving] = useState(false);
  const [saveMessage, setSaveMessage] = useState<string | null>(null);

  const sectionHelperText: Record<string, string> = {
    "API Keys & Services": "Provide service credentials and endpoint values. Use least-privilege keys where possible.",
    "Region Coverage": "Choose coverage defaults used by warning, insight, and graph views.",
    "Scheduler": "Control cadence and processing windows for automated runs.",
    "Alerting": "Define channels and thresholds for operational notifications.",
  };

  const handleToggle = (sectionLabel: string, itemKey: string) => {
    setSections((prev) =>
      prev.map((sec) =>
        sec.label === sectionLabel
          ? {
              ...sec,
              items: sec.items.map((item) =>
                item.key === itemKey && item.type === "toggle"
                  ? { ...item, value: item.value === "true" ? "false" : "true" }
                  : item
              ),
            }
          : sec
      )
    );
  };

  const handleText = (sectionLabel: string, itemKey: string, value: string) => {
    setSections((prev) =>
      prev.map((sec) =>
        sec.label === sectionLabel
          ? { ...sec, items: sec.items.map((item) => (item.key === itemKey ? { ...item, value } : item)) }
          : sec
      )
    );
  };

  const handleSelect = (sectionLabel: string, itemKey: string, value: string) => {
    setSections((prev) =>
      prev.map((sec) =>
        sec.label === sectionLabel
          ? { ...sec, items: sec.items.map((item) => (item.key === itemKey ? { ...item, value } : item)) }
          : sec
      )
    );
  };

  const validationErrors = sections.flatMap((sec) =>
    sec.items
      .filter((item) => item.type === "text")
      .map((item) => {
        const value = item.value.trim();
        if (value.length > 0) {
          return { key: `${sec.label}:${item.key}`, message: "" };
        }
        return {
          key: `${sec.label}:${item.key}`,
          message: `${item.key} is required. Provide a non-empty value before saving.`,
        };
      })
  );
  const hasErrors = validationErrors.some((e) => e.message.length > 0);

  /* Icons for each section */
  const sectionIcons: Record<string, React.ReactNode> = {
    "API Keys & Services": <Database className="h-3.5 w-3.5" />,
    "Region Coverage": <Globe className="h-3.5 w-3.5" />,
    "Scheduler": <Clock className="h-3.5 w-3.5" />,
    "Alerting": <Radio className="h-3.5 w-3.5" />,
  };

  return (
    <div>
      <PageHeader
        title="Settings"
        subtitle="API keys, region coverage, watchlist, and scheduler controls."
        icon={<Database className="h-4 w-4" />}
      />

      {/* ─── KPI Summary ─────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard label="API Keys" value="5" icon={<Database className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Active Regions" value="6" icon={<Globe className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Scheduler Jobs" value="5" icon={<Clock className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Alerting" value="On" icon={<Radio className="h-3.5 w-3.5" />} accent="#D62D2D" />
      </div>

      {/* ─── Settings Sections ───────────────────────────────── */}
      <div className="grid gap-6 lg:grid-cols-2">
        {sections.map((sec) => (
          <SurfaceCard key={sec.label} title={sec.label} icon={sectionIcons[sec.label]}>
            <p className="mb-3 text-xs text-muted-foreground">{sectionHelperText[sec.label]}</p>
            <div className="divide-y divide-border">
              {sec.items.map((item) => (
                <div key={item.key} className="flex items-center justify-between py-3">
                  <div className="min-w-0 pr-3">
                    <label htmlFor={`${sec.label}-${item.key}`} className="text-sm font-bold">
                      {item.key}
                    </label>
                    {item.type === "text" && item.value.trim().length === 0 && (
                      <p className="mt-1 text-xs text-destructive" role="alert">
                        {item.key} is required. Provide a non-empty value.
                      </p>
                    )}
                  </div>

                  {/* Toggle */}
                  {item.type === "toggle" && (
                    <div className="relative inline-flex items-center justify-center min-h-[44px] min-w-[44px]">
                      <button
                        id={`${sec.label}-${item.key}`}
                        type="button"
                        role="switch"
                        aria-checked={item.value === "true"}
                        aria-label={item.key}
                        onClick={() => handleToggle(sec.label, item.key)}
                        className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full transition-colors ${
                          item.value === "true" ? "bg-primary" : "bg-muted"
                        }`}
                      >
                        <span
                          className={`pointer-events-none absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform ${
                            item.value === "true" ? "translate-x-4" : "translate-x-0"
                          }`}
                        />
                      </button>
                    </div>
                  )}

                  {/* Text */}
                  {item.type === "text" && (
                    <input
                      id={`${sec.label}-${item.key}`}
                      type={sec.label.includes("API") ? "password" : "text"}
                      value={item.value}
                      onChange={(e) => handleText(sec.label, item.key, e.target.value)}
                      aria-label={item.key}
                      className="h-7 w-48 rounded border border-border bg-background px-2 text-xs"
                    />
                  )}

                  {/* Select */}
                  {item.type === "select" && (
                    <select
                      id={`${sec.label}-${item.key}`}
                      value={item.value}
                      onChange={(e) => handleSelect(sec.label, item.key, e.target.value)}
                      aria-label={item.key}
                      className="h-7 w-40 rounded border border-border bg-background px-2 text-xs"
                    >
                      {item.options?.map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  )}
                </div>
              ))}
            </div>
          </SurfaceCard>
        ))}
      </div>

      {/* ─── Save Button ─────────────────────────────────────── */}
      <div className="mt-6 flex justify-end">
        <p className="sr-only" aria-live="polite">
          {saveMessage ?? ""}
        </p>
        <button
          type="button"
          disabled={isSaving || hasErrors}
          onClick={async () => {
            if (hasErrors) {
              setSaveMessage("Cannot save settings due to validation errors.");
              return;
            }
            setIsSaving(true);
            setSaveMessage("Saving settings");
            await new Promise((resolve) => setTimeout(resolve, 350));
            // TODO: POST to /api/settings in production
            console.info("[ApexIntel] Settings saved (local state only)", sections);
            setIsSaving(false);
            setSaveMessage("Settings saved successfully");
          }}
          className="inline-flex min-h-[40px] items-center gap-2 rounded border border-border bg-primary px-4 py-2 text-xs font-bold uppercase text-primary-foreground hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {isSaving ? <Clock className="h-3.5 w-3.5 animate-spin" /> : <Check className="h-3.5 w-3.5" />}
          {isSaving ? "Saving..." : "Save Changes"}
        </button>
      </div>

      {hasErrors && (
        <div className="mt-3 rounded-sm border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">
          Some settings are invalid. Fix inline field messages before saving.
        </div>
      )}
    </div>
  );
}
