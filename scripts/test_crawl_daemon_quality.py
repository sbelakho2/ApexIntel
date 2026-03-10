import unittest

from scripts import crawl_daemon


class CrawlDaemonQualityTests(unittest.TestCase):
    def test_score_story_veracity_counts_primary_source_domain(self):
        result = crawl_daemon.score_story_veracity(
            story_text="The company signed a contract valued at $5 million and launched a new facility.",
            corroborating_articles=[
                {
                    "url": "https://example.org/report",
                    "text": "A second outlet reported the company executed deliveries and shipped 120 units.",
                }
            ],
            primary_urls=["https://example.com/article"],
        )

        self.assertEqual(result["classification"], "likely")
        self.assertIn("Reported by 2 independent sources", result["reasoning"])
        self.assertIn("Includes 2 non-social reporting sources", result["reasoning"])

    def test_score_story_veracity_penalizes_social_only_echo(self):
        result = crawl_daemon.score_story_veracity(
            story_text="Analysts say the company may soon expand and could begin scaling AI production.",
            corroborating_articles=[
                {
                    "url": "https://mastodon.social/@analyst/123",
                    "text": "Another post says the company may soon expand and could begin scaling AI production.",
                },
                {
                    "url": "https://mstdn.social/@watcher/456",
                    "text": "A third post repeats that the company may soon expand and could begin scaling AI production.",
                },
            ],
            primary_urls=["https://mastodon.green/@source/789"],
        )

        self.assertIn("social-only", result["reasoning"])
        self.assertIn(result["classification"], {"unverified", "contradicted", "posturing"})
        self.assertFalse(
            crawl_daemon._has_non_social_evidence_url(
                [
                    "https://mastodon.social/@analyst/123",
                    "https://mstdn.social/@watcher/456",
                ]
            )
        )

    def test_score_story_veracity_promotes_multi_source_non_social_early_report(self):
        result = crawl_daemon.score_story_veracity(
            story_text="European officials filed updated transit guidance and confirmed there are no immediate oil supply concerns after the pipeline interruption.",
            corroborating_articles=[
                {
                    "url": "https://greennuclear.online/energy/eu-pipeline-update",
                    "text": "A second report confirmed there are no immediate oil supply concerns following the transit interruption and noted updated routing guidance was published.",
                },
                {
                    "url": "https://energyintel.example/news/eu-transit-guidance",
                    "text": "A third report said officials submitted updated transit guidance while confirming no immediate supply disruption.",
                },
            ],
            primary_urls=["https://respublicae.eu/europe/pipeline-interruption-update"],
        )

        self.assertIn(result["classification"], {"likely", "verified"})
        self.assertIn("Includes 3 non-social reporting sources", result["reasoning"])
        self.assertTrue(
            crawl_daemon._has_non_social_evidence_url(
                [
                    "https://respublicae.eu/europe/pipeline-interruption-update",
                    "https://greennuclear.online/energy/eu-pipeline-update",
                ]
            )
        )

    def test_veracity_confidence_gate_blocks_unverified_rows(self):
        self.assertFalse(
            crawl_daemon._passes_confidence_gate(
                "veracity_analysis", 0.58, ["cross_reference", "veracity", "unverified"]
            )
        )
        self.assertFalse(
            crawl_daemon._passes_confidence_gate(
                "veracity_analysis", 0.52, ["cross_reference", "veracity", "contradicted"]
            )
        )

    def test_veracity_confidence_gate_keeps_likely_rows(self):
        self.assertTrue(
            crawl_daemon._passes_confidence_gate(
                "veracity_analysis", 0.61, ["cross_reference", "veracity", "likely"]
            )
        )
        self.assertIsNone(
            crawl_daemon._confidence_gate_reason(
                "veracity_analysis", 0.61, ["cross_reference", "veracity", "likely"]
            )
        )

    def test_shared_quality_gate_rejects_template_verbiage(self):
        title = "Intelligence Veracity: NVIDIA"
        summary = (
            "Additional source reporting: source one. Signal themes detected: gpu, procurement. "
            "Assessment: MODERATE-HIGH CONFIDENCE. Actionable: move now."
        )

        self.assertFalse(
            crawl_daemon._passes_shared_quality_gate(title, summary, "veracity_analysis")
        )

    def test_duplicate_paragraphs_are_collapsed_before_persistence(self):
        paragraph = (
            "European Commission is appearing in 4 recent reports across 2 independent sources. "
            "The reported development centers on market activity, with the clearest source language "
            "stating that The European Commission and EU countries confirm there are no immediate oil "
            "supply concerns following the interruption of transit via the Druzhba pipeline. Coverage "
            "from respublicae.eu, greennuclear.online is being compared for corroboration. The current "
            "read is likely at roughly 52% confidence because Reported by 2 independent sources; "
            "Includes 2 non-social reporting sources. If this matters commercially or operationally, "
            "keep it on an active watchlist and look for formal confirmation before making a hard commitment."
        )
        duplicated = f"{paragraph}\n\n{paragraph}"

        self.assertEqual(crawl_daemon._collapse_duplicate_paragraphs(duplicated), paragraph)

    def test_veracity_builder_avoids_template_sections(self):
        title, summary = crawl_daemon.build_veracity_title_and_summary(
            entity_name="NVIDIA",
            classification="likely",
            signal_types=["gpu_expansion", "procurement"],
            source_themes=[
                ("reuters.com", "NVIDIA suppliers are discussing fresh GPU capacity commitments for 2026."),
                ("digitimes.com", "Contract manufacturing partners reported follow-on procurement activity tied to accelerator demand."),
            ],
            topic_keywords=["GPU", "procurement", "supply chain"],
            veracity_result={
                "source_count": 2,
                "action_signals": 3,
                "posturing_signals": 1,
                "veracity_score": 0.74,
                "reasoning": "reported by 2 independent sources and supported by procurement evidence",
                "action_evidence": ["supplier commitments", "procurement activity"],
                "category": "demand_procurement",
            },
            report_count=3,
        )

        lower_title = title.lower()
        lower_summary = summary.lower()
        self.assertNotIn("intelligence veracity:", lower_title)
        self.assertNotIn("additional source reporting:", lower_summary)
        self.assertNotIn("signal themes detected:", lower_summary)
        self.assertNotIn("assessment:", lower_summary)
        self.assertNotIn("actionable:", lower_summary)
        self.assertTrue(
            crawl_daemon._passes_shared_quality_gate(title, summary, "veracity_analysis")
        )


if __name__ == "__main__":
    unittest.main()