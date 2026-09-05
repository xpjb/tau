package app.tau

import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.WindowState
import androidx.compose.ui.window.application
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.swing.Swing
import org.jetbrains.compose.resources.painterResource
import tau.composeapp.generated.resources.Res
import tau.composeapp.generated.resources.tau_icon

fun main() {
    PlatformServices.installCrashHandler()
    val controller = TauController(Dispatchers.Swing)
    application {
        Window(
            onCloseRequest = { controller.dispose().invokeOnCompletion { exitApplication() } },
            title = "Tau",
            icon = painterResource(Res.drawable.tau_icon),
            state = WindowState(size = DpSize(1100.dp, 760.dp)),
        ) {
            TauApp(controller)
        }
    }
}
