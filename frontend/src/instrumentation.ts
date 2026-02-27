/**
 * Next.js Instrumentation - runs once when the server starts.
 * Used to validate environment configuration before accepting requests.
 * 
 * @see https://nextjs.org/docs/app/building-your-application/optimizing/instrumentation
 */

import { validateAuthConfig } from "@/lib/auth";

export async function register() {
  // Only validate on server-side
  if (typeof window === "undefined") {
    console.log("🔧 ApexIntel: Validating configuration...");
    
    // Validate auth configuration
    const authErrors = validateAuthConfig();
    
    // Validate API key for backend communication
    if (!process.env.APEX_API_KEY) {
      authErrors.push("APEX_API_KEY is not set (required for backend communication)");
    }
    
    // In production, fail hard if configuration is invalid
    if (authErrors.length > 0) {
      console.error("❌ ApexIntel configuration errors:");
      authErrors.forEach((err) => console.error(`   - ${err}`));
      
      if (process.env.NODE_ENV === "production") {
        console.error("\n🛑 FATAL: Refusing to start in production with invalid configuration.");
        console.error("   Fix the errors above and restart the server.\n");
        process.exit(1);
      } else {
        console.warn("\n⚠️  WARNING: Running with invalid configuration in development mode.");
        console.warn("   Some features may not work correctly.\n");
      }
    } else {
      console.log("✅ ApexIntel: Configuration validated successfully.");
    }
    
    // Log startup info (non-sensitive)
    console.log(`📍 Backend URL: ${process.env.APEX_API_BASE_URL || "http://127.0.0.1:8080"}`);
    console.log(`🔐 Admin user: ${process.env.APEX_ADMIN_USERNAME || "(not set)"}`);
  }
}
