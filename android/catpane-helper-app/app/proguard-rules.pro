# CatPane Helper – ProGuard rules.
# Keep the VPN service so the system can resolve it from the manifest.
-keep class dev.catpane.helper.ThrottleVpnService { *; }
-keep class dev.catpane.helper.ControlProtocol** { *; }
