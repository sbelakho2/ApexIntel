-- ─────────────────────────────────────────────────────────────────────────────
-- ApexIntel: Fix Evidence URLs for Existing Insights
--
-- Root causes addressed:
--   1. Generic page URLs stored as sources (/about, /careers, bare homepages)
--   2. Single-segment path section pages (/press, /news) used as evidence
--   3. Duplicate URLs across multiple insight records
--   4. Irrelevant HR/legal documents included as evidence
--
-- Run with:
--   sudo -u postgres psql apexintel -f /opt/apexintel/scripts/fix_evidence_urls.sql
-- ─────────────────────────────────────────────────────────────────────────────

BEGIN;

-- ─── Helper: remove generic/irrelevant URLs from a text[] ────────────────────
CREATE OR REPLACE FUNCTION clean_evidence_urls(urls text[]) RETURNS text[] AS $$
DECLARE
    result    text[] := '{}';
    url       text;
    path_part text;
    last_seg  text;
    seg_count int;
    generic   text[] := ARRAY[
        'about', 'about_us', 'aboutus', 'careers', 'career', 'jobs',
        'job_openings', 'open_positions', 'sustainability', 'esg',
        'contact', 'contact_us', 'team', 'leadership', 'management',
        'investors', 'investor_relations', 'ir', 'products', 'solutions',
        'capabilities', 'services', 'home', 'index', 'cookies', 'privacy',
        'privacy_policy', 'legal', 'terms', 'terms_of_use', 'sitemap',
        'search', 'events', 'tradeshows', 'trade_shows'
    ];
BEGIN
    IF urls IS NULL THEN RETURN result; END IF;

    FOREACH url IN ARRAY urls LOOP
        -- Skip NULL/empty
        CONTINUE WHEN url IS NULL OR url = '';

        -- Skip bare homepages  (https://example.com  or  https://example.com/)
        CONTINUE WHEN url ~ '^https?://[^/]+/?$';

        -- Extract URL path (remove scheme+host, then query/fragment)
        path_part := regexp_replace(url, '^https?://[^/]+', '');
        path_part := regexp_replace(path_part, '[?#].*$', '');
        path_part := rtrim(path_part, '/');

        -- Count non-empty path segments
        seg_count := (
            SELECT COUNT(*)
            FROM unnest(string_to_array(path_part, '/')) AS s
            WHERE s <> ''
        );

        -- Require at least 2 segments — single-segment paths are section indexes
        -- (/about, /news, /press, /products, /careers, etc.)
        CONTINUE WHEN seg_count < 2;

        -- Get the last non-empty segment
        last_seg := regexp_replace(path_part, '^.*/', '');
        last_seg := lower(last_seg);
        -- Normalise separators to underscores
        last_seg := translate(last_seg, '-.', '__');
        -- Strip common file extensions so 'about.html' is treated as 'about'
        last_seg := regexp_replace(last_seg, '\.(pdf|html?|aspx?|php|jsp)$', '');

        -- Skip if last segment is a known generic word
        CONTINUE WHEN last_seg = ANY(generic);

        result := result || url;
    END LOOP;

    RETURN result;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- ─── Step 1: Strip generic URLs from all insights ─────────────────────────────
UPDATE insights
SET    evidence_urls = clean_evidence_urls(evidence_urls),
       updated_at    = NOW()
WHERE  array_length(evidence_urls, 1) > 0;

-- ─── Step 2: Remove irrelevant HR/legal documents that survived step 1 ─────────
-- The nemco Job-Applicant Privacy Notice PDF is not an intelligence source.
UPDATE insights
SET    evidence_urls = array_remove(
           evidence_urls,
           'https://www.nemco.co.uk/wp-content/uploads/2021/05/Job-Applicant-Privacy-Notice.pdf'
       ),
       updated_at    = NOW()
WHERE  evidence_urls @> ARRAY[
           'https://www.nemco.co.uk/wp-content/uploads/2021/05/Job-Applicant-Privacy-Notice.pdf'
       ];

-- ─── Step 3: Remove the temporary helper function ─────────────────────────────
DROP FUNCTION IF EXISTS clean_evidence_urls(text[]);

COMMIT;

-- ─── Verification output ──────────────────────────────────────────────────────
\echo ''
\echo '=== Evidence URL Counts by Type After Patch ==='
SELECT
    insight_type,
    COUNT(*)                                                             AS total,
    COUNT(*) FILTER (WHERE array_length(evidence_urls, 1) > 0)         AS with_urls,
    COUNT(*) FILTER (WHERE coalesce(array_length(evidence_urls,1),0)=0) AS empty_urls
FROM insights
GROUP BY insight_type
ORDER BY insight_type;

\echo ''
\echo '=== All Insights After Patch ==='
SELECT
    title,
    insight_type,
    array_length(evidence_urls, 1) AS url_count,
    evidence_urls
FROM insights
ORDER BY insight_type, title;
