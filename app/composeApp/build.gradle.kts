import java.util.Properties
import org.gradle.api.file.DuplicatesStrategy
import org.gradle.api.tasks.Sync
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("org.jetbrains.kotlin.multiplatform")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
    id("org.jetbrains.compose")
    id("com.android.application")
}

kotlin {
    androidTarget {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }
    jvm("desktop") {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }

    sourceSets {
        commonMain.dependencies {
            implementation(compose.runtime)
            implementation(compose.foundation)
            implementation(compose.material3)
            implementation(compose.ui)
            implementation(compose.components.resources)
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
            implementation("io.ktor:ktor-client-core:3.0.3")
            implementation("io.ktor:ktor-client-websockets:3.0.3")
        }
        androidMain.dependencies {
            implementation("androidx.activity:activity-compose:1.10.0")
            implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
            implementation("io.ktor:ktor-client-okhttp:3.0.3")
        }
        val desktopMain by getting {
            dependencies {
                implementation(compose.desktop.currentOs)
                implementation("io.ktor:ktor-client-cio:3.0.3")
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.9.0")
            }
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
        }
    }
}

val localProperties = Properties().apply {
    val propertiesFile = rootProject.file("local.properties")
    if (propertiesFile.isFile) propertiesFile.inputStream().use(::load)
}
val releaseStoreFile = localProperties.getProperty("tau.signing.storeFile")
val releaseStorePassword = localProperties.getProperty("tau.signing.storePassword")
val releaseKeyAlias = localProperties.getProperty("tau.signing.keyAlias")
val releaseKeyPassword = localProperties.getProperty("tau.signing.keyPassword")
val hasReleaseSigning = listOf(
    releaseStoreFile,
    releaseStorePassword,
    releaseKeyAlias,
    releaseKeyPassword,
).all { !it.isNullOrBlank() }

android {
    namespace = "app.tau"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.tau"
        minSdk = 26
        targetSdk = 35
        versionCode = 4
        versionName = "0.3.0"
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("tauRelease") {
                storeFile = rootProject.file(checkNotNull(releaseStoreFile))
                storePassword = checkNotNull(releaseStorePassword)
                keyAlias = checkNotNull(releaseKeyAlias)
                keyPassword = checkNotNull(releaseKeyPassword)
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("tauRelease")
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

val windowsSkiko by configurations.creating {
    isCanBeConsumed = false
    isCanBeResolved = true
    isTransitive = false
}

dependencies {
    windowsSkiko("org.jetbrains.skiko:skiko-awt-runtime-windows-x64:0.8.18")
}

tasks.register<Sync>("prepareWindowsApp") {
    dependsOn(tasks.named("proguardReleaseJars"))
    into(layout.buildDirectory.dir("windows/app/lib"))
    from(layout.buildDirectory.dir("compose/tmp/main-release/proguard")) {
        exclude("skiko-awt-runtime-linux-*.jar")
    }
    from(windowsSkiko)
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
}

compose.desktop {
    application {
        mainClass = "app.tau.MainKt"
    }
}
