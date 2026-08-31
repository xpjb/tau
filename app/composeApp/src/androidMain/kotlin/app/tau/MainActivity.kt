package app.tau

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.remember
import kotlinx.coroutines.Dispatchers

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        TauAndroidContext.initialize(applicationContext)
        PlatformServices.installCrashHandler()
        super.onCreate(savedInstanceState)
        setContent {
            TauApp(remember { TauController(Dispatchers.Main.immediate) })
        }
    }
}
