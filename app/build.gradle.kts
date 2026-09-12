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

val selectionPatched = Attribute.of("app.tau.selection-patched", Boolean::class.javaObjectType)
subprojects {
    configurations.configureEach {
        attributes.attribute(selectionPatched, true)
    }
    dependencies {
        for (module in listOf("org.jetbrains.compose.foundation:foundation-desktop", "androidx.compose.foundation:foundation-android")) {
            components.withModule(module) {
                if (id.version != "1.12.0") throw GradleException("Review the Compose selection patch before changing $id")
            }
        }
        artifactTypes.maybeCreate("aar")
        artifactTypes.configureEach {
            if (name == "jar" || name == "aar") attributes.attribute(selectionPatched, false)
        }
        for (type in listOf("jar", "aar")) {
            registerTransform(ComposeSelectionPatch::class) {
                from.attribute(selectionPatched, false).attribute(ARTIFACT_TYPE_ATTRIBUTE, type)
                to.attribute(selectionPatched, true).attribute(ARTIFACT_TYPE_ATTRIBUTE, type)
            }
        }
    }
}
