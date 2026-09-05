import org.gradle.api.file.DuplicatesStrategy
import org.gradle.api.tasks.JavaExec
import org.gradle.api.tasks.Sync
import org.gradle.api.tasks.testing.Test
import org.gradle.jvm.toolchain.JavaLanguageVersion
import org.gradle.jvm.toolchain.JavaToolchainService
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("org.jetbrains.kotlin.multiplatform")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
    id("org.jetbrains.compose")
    id("com.android.kotlin.multiplatform.library")
}

kotlin {
    jvmToolchain(21)

    android {
        namespace = "app.tau.shared"
        compileSdk = 37
        minSdk = 26
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }
    jvm("desktop") {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_21)
    }

    sourceSets {
        commonMain.dependencies {
            implementation("org.jetbrains.compose.runtime:runtime:1.12.0")
            implementation("org.jetbrains.compose.foundation:foundation:1.12.0")
            implementation("org.jetbrains.compose.material3:material3:1.9.0")
            implementation("org.jetbrains.compose.ui:ui:1.12.0")
            implementation("org.jetbrains.compose.components:components-resources:1.12.0")
            implementation("com.mikepenz:multiplatform-markdown-renderer:0.45.0")
            implementation("io.coil-kt.coil3:coil-compose:3.6.1")
            implementation("io.coil-kt.coil3:coil-network-ktor3:3.6.1")
            implementation("org.jetbrains:markdown:0.7.9")
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")
            implementation("io.ktor:ktor-client-core:3.5.2")
            implementation("io.ktor:ktor-client-websockets:3.5.2")
            implementation("com.squareup.okio:okio:3.18.1")
            implementation("androidx.sqlite:sqlite-bundled:2.7.0")
        }
        androidMain.dependencies {
            implementation("androidx.activity:activity-compose:1.13.0")
            implementation("androidx.core:core:1.18.0")
            implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
            implementation("io.ktor:ktor-client-okhttp:3.5.2")
        }
        val desktopMain by getting {
            dependencies {
                implementation(compose.desktop.currentOs)
                implementation("io.ktor:ktor-client-cio:3.5.2")
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.11.0")
            }
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
        }
        val desktopTest by getting {
            dependencies {
                implementation("io.ktor:ktor-server-cio-jvm:3.5.2")
                implementation("io.ktor:ktor-server-websockets-jvm:3.5.2")
            }
        }
    }
}

val windowsSkiko by configurations.creating {
    isCanBeConsumed = false
    isCanBeResolved = true
    isTransitive = false
}

dependencies {
    windowsSkiko("org.jetbrains.skiko:skiko-awt-runtime-windows-x64:0.150.1")
}

val java21 = extensions.getByType<JavaToolchainService>().launcherFor {
    languageVersion.set(JavaLanguageVersion.of(21))
}

tasks.withType<JavaExec>().configureEach {
    javaLauncher.set(java21)
}
tasks.withType<Test>().configureEach {
    javaLauncher.set(java21)
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
        javaHome = java21.get().metadata.installationPath.asFile.absolutePath
        buildTypes.release.proguard {
            configurationFiles.from(project.file("proguard-desktop.pro"))
        }
    }
}
