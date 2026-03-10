#!/usr/bin/env python3

import asyncio
import importlib.util
import os
import sys

import asyncpg


DEFAULT_TARGETS = [
    "BAE Systems",
    "Key Tronic Corporation",
    "NVIDIA Corporation",
]


def load_crawl_daemon():
    script_path = "/opt/apexintel/scripts/crawl_daemon.py"
    if not os.path.exists(script_path):
        script_path = os.path.join(os.path.dirname(__file__), "crawl_daemon.py")
    spec = importlib.util.spec_from_file_location("crawl_daemon", script_path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


async def main() -> int:
    crawl_daemon = load_crawl_daemon()
    target_mode = len(sys.argv) > 1 and sys.argv[1] == "--top"
    top_limit = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[1] == "--top" else 10
    pool = await asyncpg.create_pool(crawl_daemon.DATABASE_URL, min_size=1, max_size=2, command_timeout=60)
    try:
        recent_obs = await pool.fetch(
            """
            SELECT o.id, o.observation_type, o.entity_id, o.entity_type, o.value, o.provenance, o.confidence,
                   COALESCE(c.name, '') as company_name
            FROM observations o
            LEFT JOIN companies c ON c.id = o.entity_id
            WHERE o.ts_utc > NOW() - INTERVAL '6 hours'
            AND o.observation_type IN ('CompetitorEvent', 'WebChange', 'SocialPost')
            AND o.confidence >= 0.5
            ORDER BY o.ts_utc DESC
            LIMIT 250
            """
        )

        entity_clusters = {}
        for obs in recent_obs:
            entity_key = obs["company_name"] or "unaffiliated"
            entity_clusters.setdefault(entity_key, [])
            value = crawl_daemon.json.loads(obs["value"]) if isinstance(obs["value"], str) else obs["value"]
            provenance = crawl_daemon.json.loads(obs["provenance"]) if isinstance(obs["provenance"], str) else obs["provenance"]
            entity_clusters[entity_key].append(
                {
                    "entity_id": str(obs["entity_id"]) if obs["entity_id"] else None,
                    "text": value.get("excerpt", value.get("text", "")),
                    "url": value.get("url", provenance.get("url", "")),
                    "signals": value.get("signals", []),
                    "platform": provenance.get("platform", value.get("source", "web")),
                }
            )

        scored_rows = []
        for entity, obs_list in entity_clusters.items():
            if target_mode:
                if entity == "unaffiliated":
                    continue
                lower = entity.lower()
                if lower.startswith("government of ") or lower.startswith("ministry of ") or lower.startswith("republic of "):
                    continue
            elif entity not in DEFAULT_TARGETS:
                continue

            obs_list = entity_clusters.get(entity, [])
            substantive_obs = [
                obs
                for obs in obs_list
                if len(obs.get("text", "")) >= 80 and not crawl_daemon._is_boilerplate_excerpt(obs.get("text", ""))
            ]
            if len(substantive_obs) < 2:
                continue

            primary_texts = [obs["text"] for obs in substantive_obs if obs["text"]]
            combined_text = " ".join(primary_texts[:5])
            corroborating = []
            primary_url = substantive_obs[0].get("url", "")
            for obs in substantive_obs[1:]:
                if obs.get("url") != primary_url and obs.get("text", ""):
                    corroborating.append(
                        {
                            "text": obs.get("text", ""),
                            "url": obs.get("url", ""),
                            "platform": obs.get("platform", ""),
                        }
                    )

            result = crawl_daemon.score_story_veracity(
                combined_text,
                corroborating,
                primary_urls=[obs.get("url", "") for obs in substantive_obs if obs.get("url")],
            )
            signal_types = sorted({signal for obs in substantive_obs for signal in obs.get("signals", [])})
            source_themes = []
            seen_domains = set()
            for obs in substantive_obs[:6]:
                try:
                    domain = crawl_daemon.urlparse(obs.get("url", "")).netloc.replace("www.", "")
                except Exception:
                    domain = "unknown"
                if domain in seen_domains:
                    continue
                seen_domains.add(domain)
                raw = crawl_daemon._trim_excerpt_sentence(obs.get("text", ""), limit=180)
                if raw:
                    source_themes.append((domain, raw))

            title, summary = crawl_daemon.build_veracity_title_and_summary(
                entity,
                result["classification"],
                signal_types,
                source_themes,
                crawl_daemon._extract_key_topics(combined_text, entity),
                result,
                len(substantive_obs),
            )
            confidence = min(0.95, result["veracity_score"] * 0.9 + 0.05)
            confidence_reason = crawl_daemon._confidence_gate_reason(
                "veracity_analysis",
                confidence,
                ["cross_reference", "veracity", result["classification"]],
            )
            source_domains = sorted(
                {
                    crawl_daemon.urlparse(obs.get("url", "")).netloc.replace("www.", "")
                    for obs in substantive_obs
                    if obs.get("url")
                }
            )
            non_social_domains = [
                domain for domain in source_domains if not crawl_daemon._is_social_or_low_signal_domain(domain)
            ]
            scored_rows.append(
                {
                    "entity": entity,
                    "obs": len(obs_list),
                    "substantive": len(substantive_obs),
                    "classification": result["classification"],
                    "score": result["veracity_score"],
                    "confidence": confidence,
                    "sources": result["source_count"],
                    "source_domains": source_domains,
                    "non_social_domains": non_social_domains,
                    "reasoning": result["reasoning"],
                    "title": title,
                    "gate": crawl_daemon._passes_shared_quality_gate(title, summary, "veracity_analysis"),
                    "confidence_reason": confidence_reason,
                    "summary": summary[:800],
                }
            )

        if target_mode:
            scored_rows.sort(
                key=lambda row: (
                    len(row["non_social_domains"]),
                    row["confidence"],
                    row["substantive"],
                ),
                reverse=True,
            )
            scored_rows = scored_rows[:top_limit]

        for row in scored_rows:
            print(f"=== {row['entity']} ===")
            print(f"obs={row['obs']} substantive={row['substantive']}")
            print(
                f"classification={row['classification']} score={row['score']:.3f} "
                f"conf={row['confidence']:.3f} sources={row['sources']}"
            )
            print(f"domains={', '.join(row['source_domains']) or 'none'}")
            print(f"non_social_domains={', '.join(row['non_social_domains']) or 'none'}")
            print(f"reasoning={row['reasoning']}")
            print(f"title={row['title']}")
            print(f"gate={row['gate']} confidence_reason={row['confidence_reason']}")
            print(f"summary={row['summary']}")
            print()
    finally:
        await pool.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))