/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  experimental: {
    typedRoutes: true,
    instrumentationHook: true,
  },
  images: {
    remotePatterns: [
      // Restrict to known trusted domains instead of wildcard
      { protocol: 'https', hostname: 'avatars.githubusercontent.com' },
      { protocol: 'https', hostname: 'logo.clearbit.com' },
    ],
  },
  async rewrites() {
    const apiBase = process.env.APEX_API_BASE_URL || process.env.NEXT_PUBLIC_API_BASE_URL || process.env.API_BASE_URL || 'http://localhost:8080';
    return [
      // Only rewrite public endpoints directly to backend
      // Protected endpoints go through /api/proxy/* which handles auth server-side
      {
        source: '/api/health',
        destination: `${apiBase}/api/health`,
      },
      {
        source: '/api/endpoints',
        destination: `${apiBase}/api/endpoints`,
      },
      {
        source: '/ws/:path*',
        destination: `${apiBase}/ws/:path*`,
      },
    ];
  },
  async headers() {
    return [
      {
        source: '/:all*(svg|jpg|jpeg|png|webp|avif|ico|woff|woff2|ttf)',
        headers: [
          {
            key: 'Cache-Control',
            value: 'public, max-age=31536000, immutable',
          },
        ],
      },
    ];
  },
};

module.exports = nextConfig;
