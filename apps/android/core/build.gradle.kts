plugins {
    kotlin("jvm")
    kotlin("plugin.serialization")
}

kotlin { jvmToolchain(21) }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    // SSH for reaching computers that only listen on loopback. Bouncy Castle supplies Ed25519 and
    // X25519 on Android, whose JCA lacks them.
    implementation("com.github.mwiede:jsch:0.2.21")
    implementation("org.bouncycastle:bcprov-jdk18on:1.78.1")
    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
}

tasks.test {
    useJUnit()
    System.getenv("ZUSE_NATIVE_JNI_DIR")?.let { systemProperty("java.library.path", it) }
}

kotlin.compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)

tasks.withType<JavaCompile>().configureEach { options.release.set(17) }

dependencies {
    testImplementation("com.squareup.okhttp3:okhttp-tls:4.12.0")
    testImplementation("org.apache.sshd:sshd-core:2.14.0")
    testImplementation("org.slf4j:slf4j-nop:2.0.16")
}
