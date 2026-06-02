# rustls-platform-verifier JNI (coord HTTPS in :p2p) — not visible to Proguard static analysis.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
