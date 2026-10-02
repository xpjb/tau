#!/usr/bin/env python3
"""Render the ORIGINAL Tau1 Compose download controls, not an HTML reconstruction.
Extracts the control block and theme verbatim from a pinned Git revision. Only action
callbacks/platform detection are stubbed. Uses an isolated, offline Gradle project;
never builds/launches the stable app, native transfer library or daemon.

python3 render-tau1.py /root/tau /tmp/tau1-download-reference /tmp/tau2-download-qa/tau1
"""
import json, pathlib, re, subprocess, sys, textwrap
repo, work, out = map(pathlib.Path, sys.argv[1:4])
ref = '3818579'
def source(file):
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{ref}:app/composeApp/src/commonMain/kotlin/app/tau/{file}'], text=True)
app = source('TauApp.kt')
block = app[app.index('                                                        val totalBytes = attachmentDownload?.totalBytes'):app.index('\n                                                    }\n                                                }\n                                            }\n                                                    }', app.index('val totalBytes = attachmentDownload?.totalBytes'))]
block = textwrap.dedent(block)
block = re.sub(r'onClick = \{.*?\},', 'onClick = {},', block, flags=re.S)
block = block.replace('PlatformServices.platformName == "windows"', '!mobile')
block = block.replace('TextButton(', 'TextButton(modifier = Modifier.onGloballyPositioned { buttons.add(it.boundsInWindow()) },')
theme = app[app.index('private val TauDarkColors'):app.index('\n\n@Composable\nfun TauApp')]
byte_source = source('TranscriptText.kt')
byte_source = byte_source[byte_source.index('internal fun formatByteCount'):]
# Last helper in this file; keep its original implementation, not a Python approximation.
brace = byte_source.index('{'); depth = 1; end = brace + 1
while depth:
    depth += (byte_source[end] == '{') - (byte_source[end] == '}'); end += 1
byte_source = byte_source[:end]
cases = json.loads((pathlib.Path(__file__).parent / 'cases.json').read_text())
def kt(value):
    return json.dumps(value, ensure_ascii=False).replace('$', '\\$') if value is not None else 'null'
items = []
for c in cases:
    size = str(c['size']) + 'L' if 'size' in c else 'null'
    status = {'active':'Downloading', 'failed':'Failed', 'unavailable':'Downloaded', 'cached':'Downloaded', 'saving':'Downloading', 'save-failed':'Failed', 'saved':'Downloaded', 'saved-no-cache':'Downloaded', 'missing':None, 'decode-failed':'Downloaded'}.get(c['state'])
    download = 'null' if status is None else f'AttachmentDownload(AttachmentDownloadStatus.{status}, {c.get("transferred", c.get("size", 0) if c["state"] == "saving" else 0)}L, {size}, ' + (str(c['rate'])+'L' if c.get('rate') else 'null') + f', {"Any()" if c["state"] in ("saved", "saved-no-cache") else "null"}, Failure({kt(c.get("error"))}))'
    items.append(f'Case({kt(c["id"])}, {kt(c["label"])}, Attachment({kt(c.get("name", "preview.png" if c.get("image") else "release-notes.pdf"))}, {size}), {download})')
code = '''
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInWindow
import kotlinx.coroutines.*
import kotlinx.coroutines.swing.Swing
import org.jetbrains.skia.EncodedImageFormat
import java.io.File
import kotlin.math.*
'''+theme+'\n'+byte_source+'''
private val AttachmentControlHeight = 68.dp
enum class AttachmentDownloadStatus { Downloading, Downloaded, Failed }
data class Failure(val message: String?)
data class AttachmentDownload(val status: AttachmentDownloadStatus, val transferredBytes: Long, val totalBytes: Long?, val bytesPerSecond: Long?, val saved: Any?, val failure: Failure?)
data class Attachment(val fileName: String, val size: Long?)
data class Case(val id: String, val label: String, val attachment: Attachment, val download: AttachmentDownload?)
@Composable
fun OriginalControl(attachment: Attachment, attachmentDownload: AttachmentDownload?, mobile: Boolean, buttons: MutableList<Rect>) {
'''+block+'''
}
@OptIn(ExperimentalComposeUiApi::class)
fun main(args: Array<String>) = runBlocking(Dispatchers.Swing) {
    val cases = listOf(
'''+',\n'.join(items)+'''
    )
    val out = File(args[0]); out.mkdirs()
    for ((profile, width, scale) in listOf(Triple("desktop",552,1f), Triple("sidebar",320,1f), Triple("phone",360,1f), Triple("phone-2x5",900,2.5f))) {
        for (case in cases) {
            val buttons = mutableListOf<Rect>()
            val scene = ImageComposeScene(width, (148*scale).toInt(), density=Density(scale), coroutineContext=Dispatchers.Swing) {
                MaterialTheme(colorScheme=TauDarkColors) {
                    CompositionLocalProvider(LocalContentColor provides TauDarkColors.onSurface) {
                    Column(Modifier.fillMaxSize().background(TauDarkColors.surface).padding(12.dp)) {
                        Text(case.label, style=MaterialTheme.typography.labelMedium, color=TauDarkColors.onSurfaceVariant, modifier=Modifier.height(32.dp))
                        Box(Modifier.fillMaxWidth().background(TauDarkColors.surfaceVariant).padding(horizontal=14.dp)) {
                            OriginalControl(case.attachment, case.download, profile.startsWith("phone"), buttons)
                        }
                    }
                    }
                }
            }
            try {
                scene.render(0L).close()
                var time = 250_000_000L
                fun capture(suffix: String) {
                    scene.render(time).use { image -> image.encodeToData(EncodedImageFormat.PNG)!!.use { data ->
                        File(out,"$profile-${case.id}$suffix.png").writeBytes(data.bytes)
                    } }
                    time += 300_000_000L
                }
                capture("")
                for ((index, button) in buttons.distinct().withIndex()) {
                    scene.sendPointerEvent(PointerEventType.Move, button.center)
                    scene.render(time).close(); time += 300_000_000L
                    capture("-hover-$index")
                    scene.sendPointerEvent(PointerEventType.Press, button.center)
                    scene.render(time).close(); time += 100_000_000L
                    capture("-pressed-$index")
                    scene.sendPointerEvent(PointerEventType.Release, button.center)
                    scene.sendPointerEvent(PointerEventType.Move, Offset.Zero)
                    scene.render(time).close(); time += 300_000_000L
                }
            } finally { scene.close() }
        }
    }
}
'''
(work / 'src/main/kotlin').mkdir(parents=True, exist_ok=True)
(work / 'src/main/kotlin/Main.kt').write_text(code)
(work / 'settings.gradle.kts').write_text('''pluginManagement { resolutionStrategy { eachPlugin { if (requested.id.id == "org.jetbrains.kotlin.jvm") useModule("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.10") } }; repositories { mavenCentral(); gradlePluginPortal(); google() } }
dependencyResolutionManagement { repositories { mavenCentral(); google() } }
rootProject.name="tau1-download-reference"
''')
(work / 'build.gradle.kts').write_text('''plugins {
    kotlin("jvm") version "2.4.10"
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.10"
    id("org.jetbrains.compose") version "1.12.0"
    application
}
kotlin { jvmToolchain(21) }
dependencies {
    implementation(compose.desktop.currentOs)
    implementation("org.jetbrains.compose.material3:material3:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.11.0")
}
application { mainClass.set("MainKt") }
''')
(work / 'gradle.properties').write_text('org.gradle.jvmargs=-Xmx1536m -XX:ActiveProcessorCount=2\norg.gradle.workers.max=1\nkotlin.compiler.execution.strategy=in-process\n')
(out).mkdir(parents=True, exist_ok=True)
(out / 'reference.txt').write_text(f'Tau1 {ref}, original 68dp Compose download control. Action callbacks stubbed.\nTau1 has no distinct OS-saving UI; mapped to its final downloading frame.\nTau1 has no separate View icon (click its image preview). Desktop platform forced to Windows.\nImage preview is outside this control; reference captures the original control only.\n')
subprocess.run([str(repo / 'app/gradlew'), '--offline', '--no-daemon', '-p', str(work), 'run', '--args='+str(out)], check=True)
