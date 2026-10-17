# Public API is reached through Tauri's generated mobile plugin bindings.
# The vendor SDK uses reflection and EventBus subscribers internally.
-keep class com.onyx.android.sdk.** { *; }

# The SDK's transitive dependencies (Jackson, OkHttp, Joda, SLF4J) reference
# optional JVM/JSSE classes that do not exist on Android. R8 treats each
# missing class as an error in release builds; none of these code paths run.
-dontwarn java.beans.ConstructorProperties
-dontwarn java.beans.Transient
-dontwarn javax.lang.model.element.Modifier
-dontwarn org.bouncycastle.jsse.BCSSLParameters
-dontwarn org.bouncycastle.jsse.BCSSLSocket
-dontwarn org.bouncycastle.jsse.provider.BouncyCastleJsseProvider
-dontwarn org.conscrypt.Conscrypt$Version
-dontwarn org.conscrypt.Conscrypt
-dontwarn org.conscrypt.ConscryptHostnameVerifier
-dontwarn org.joda.convert.FromString
-dontwarn org.joda.convert.ToString
-dontwarn org.openjsse.javax.net.ssl.SSLParameters
-dontwarn org.openjsse.javax.net.ssl.SSLSocket
-dontwarn org.openjsse.net.ssl.OpenJSSE
-dontwarn org.slf4j.impl.StaticLoggerBinder
-dontwarn org.slf4j.impl.StaticMDCBinder
-dontwarn org.slf4j.impl.StaticMarkerBinder

# Onyx's optional fingerprint and Wi-Fi UI helpers are not used by this plugin.
-dontwarn android.hardware.fingerprint.FingerprintManager
-dontwarn com.onyx.android.sdk.BR

# Fastjson2 probes these optional Fastjson 1.x compatibility classes and catches
# their absence. The pen SDK's Fastjson2 dependency does not bundle Fastjson 1.x.
-dontwarn com.alibaba.fastjson.JSONArray
-dontwarn com.alibaba.fastjson.JSONObject
