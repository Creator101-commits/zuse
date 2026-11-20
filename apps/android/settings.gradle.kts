pluginManagement {
    repositories {
        google()
        // Google's mirror of Maven Central first: CI runners are
        // intermittently refused by Maven Central itself, and Gradle stops at
        // an error from the first repository instead of falling through.
        maven("https://maven-central.storage-download.googleapis.com/maven2/")
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        // Google's mirror of Maven Central first: CI runners are
        // intermittently refused by Maven Central itself, and Gradle stops at
        // an error from the first repository instead of falling through.
        maven("https://maven-central.storage-download.googleapis.com/maven2/")
        mavenCentral()
    }
}

rootProject.name = "ZuseAndroid"

include(":core", ":app")
