package sh.zeron.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ChatBubble
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.ShortNavigationBar
import androidx.compose.material3.ShortNavigationBarItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.runtime.remember
import sh.zeron.android.design.ZIcons
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZeronTheme

@Composable
fun ZeronRoot(model: AppModel) {
    val appearance by model.appearance.collectAsState()
    ZeronTheme(appearance) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            val client by model.client.collectAsState()
            AnimatedContent(client != null, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "root") { signedIn ->
                if (signedIn) MainNav(model) else SignInScreen(model)
            }
        }
    }
}

object Routes {
    const val HOME = "home"
    const val CHAT = "chat/{id}"
    const val NEW = "new"
    const val SETTINGS = "settings"
    fun chat(id: String) = "chat/$id"
}

@Composable
private fun MainNav(model: AppModel) {
    val nav = rememberNavController()
    LaunchedEffect(Unit) {
        when (val route = model.launch.route) {
            null, "search" -> Unit
            "new" -> nav.navigate(Routes.NEW)
            "settings" -> nav.navigate(Routes.SETTINGS)
            else -> if (route.startsWith("chat:")) nav.navigate(Routes.chat(route.removePrefix("chat:")))
        }
    }
    NavHost(nav, startDestination = Routes.HOME) {
        composable(Routes.HOME) {
            SessionsScreen(
                model,
                onOpen = { nav.navigate(Routes.chat(it)) },
                onNew = { nav.navigate(Routes.NEW) },
                onSettings = { nav.navigate(Routes.SETTINGS) },
                startSearching = model.launch.route == "search",
            )
        }
        composable(Routes.CHAT) { entry ->
            val id = entry.arguments?.getString("id") ?: return@composable
            SessionScreen(model, id, onBack = { nav.popBackStack() })
        }
        composable(Routes.NEW) {
            NewSessionScreen(model, onClose = { nav.popBackStack() }, onCreated = { id ->
                nav.popBackStack()
                nav.navigate(Routes.chat(id))
            })
        }
        composable(Routes.SETTINGS) { SettingsScreen(model, onBack = { nav.popBackStack() }) }
    }
}
