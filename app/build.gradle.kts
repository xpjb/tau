import org.gradle.api.attributes.Attribute
import org.gradle.api.artifacts.type.ArtifactTypeDefinition.ARTIFACT_TYPE_ATTRIBUTE

plugins {
    id("com.android.application") version "9.1.1" apply false
    id("com.android.kotlin.multiplatform.library") version "9.1.1" apply false
    id("org.jetbrains.kotlin.multiplatform") version "2.4.10" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.10" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.4.10" apply false
    id("org.jetbrains.compose") version "1.12.0" apply false
}

val selectionPatch = Attribute.of("app.tau.selection-patch", String::class.java)
subprojects {
    configurations.configureEach {
        attributes.attribute(selectionPatch, "patched")
    }
    dependencies {
        for (module in listOf("org.jetbrains.compose.foundation:foundation-desktop", "androidx.compose.foundation:foundation-android")) {
            components.withModule(module) {
                if (id.version != "1.12.0") throw GradleException("Review the Compose selection patch before changing $id")
            }
        }
        artifactTypes.maybeCreate("aar")
        artifactTypes.configureEach {
            if (name == "jar" || name == "aar") attributes.attribute(selectionPatch, "raw-$name")
        }
        for (type in listOf("jar", "aar")) {
            registerTransform(ComposeSelectionPatch::class) {
                from.attribute(selectionPatch, "raw-$type").attribute(ARTIFACT_TYPE_ATTRIBUTE, type)
                to.attribute(selectionPatch, "patched").attribute(ARTIFACT_TYPE_ATTRIBUTE, type)
            }
        }
    }
}
