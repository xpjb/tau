package app.tau

import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.WindowState
import androidx.compose.ui.window.application

fun main() {
    PlatformServices.installCrashHandler()
    application {
        Window(
            onCloseRequest = ::exitApplication,
            title = "Tau",
            state = WindowState(size = DpSize(1100.dp, 760.dp)),
        ) {
            TauApp()
        }
    }
}
