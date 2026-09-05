package app.tau

import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import kotlinx.coroutines.Dispatchers

class TauViewModel : ViewModel() {
    val controller = TauController(Dispatchers.Main.immediate)
    override fun onCleared() { controller.dispose() }
}

class MainActivity : ComponentActivity() {
    private val filePicker = registerForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments(),
        TauAndroidContext::completeFileSelection,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        TauAndroidContext.initialize(applicationContext, filePicker)
        PlatformServices.installCrashHandler()
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        super.onCreate(savedInstanceState)
        val controller = ViewModelProvider(this)[TauViewModel::class.java].controller
        setContent { TauApp(controller) }
    }

    override fun onDestroy() {
        TauAndroidContext.detach(filePicker)
        super.onDestroy()
    }
}
