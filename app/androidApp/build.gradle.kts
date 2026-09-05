import java.util.Properties

plugins {
    id("com.android.application")
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
    compileSdk = 37

    defaultConfig {
        applicationId = "app.tau"
        minSdk = 26
        targetSdk = 37
        versionCode = 13
        versionName = "0.4.5"
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

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

dependencies {
    implementation(project(":composeApp"))
    implementation("androidx.core:core:1.18.0")
}
