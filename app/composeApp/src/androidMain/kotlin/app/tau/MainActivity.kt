package app.tau

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import kotlinx.coroutines.Dispatchers

class MainActivity : ComponentActivity() {
    private lateinit var controller: TauController

    override fun onCreate(savedInstanceState: Bundle?) {
        TauAndroidContext.initialize(applicationContext)
        PlatformServices.installCrashHandler()
        super.onCreate(savedInstanceState)
        controller = TauController(Dispatchers.Main.immediate)
        setContent { TauApp(controller) }
    }
}
