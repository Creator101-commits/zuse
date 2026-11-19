# Kotlin serialization generates and retains serializers through its consumer rules.

# JNI entry point must retain its exact JVM name.
-keep class sh.zuse.core.NativePairing { *; }

# JSch instantiates its ciphers, key exchanges and signatures by class name.
-keep class com.jcraft.jsch.** { *; }
-dontwarn com.jcraft.jsch.**
-dontwarn org.bouncycastle.**
-dontwarn org.slf4j.**
-dontwarn org.apache.logging.log4j.**
-dontwarn org.newsclub.net.unix.**
-dontwarn com.sun.jna.**
