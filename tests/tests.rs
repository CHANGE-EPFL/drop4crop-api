// Main test entry point - consolidates all tests into a single crate
// This eliminates "unused" warnings for common test utilities
//
// Run tests with: cargo test --test tests -- --test-threads=1
// (sequential execution avoids database conflicts)
//
// Tests auto-detect external services and skip gracefully if unavailable:
// - S3 tests: Check for MinIO/S3 connectivity before running
// - Redis tests: Check for Redis connectivity before running
// - Keycloak tests: Check for KEYCLOAK_URL env var before running

mod common;
mod modules;
