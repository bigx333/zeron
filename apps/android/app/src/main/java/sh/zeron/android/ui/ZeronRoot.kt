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
    const val SEARCH = "search"
    fun chat(id: String) = "chat/$id"
}

@Composable
private fun MainNav(model: AppModel) {
    val nav = rememberNavController()
    LaunchedEffect(Unit) {
        when (val route = model.launch.route) {
            null -> Unit
            "new" -> nav.navigate(Routes.NEW)
            "search" -> nav.navigate(Routes.SEARCH)
            else -> if (route.startsWith("chat:")) nav.navigate(Routes.chat(route.removePrefix("chat:")))
        }
    }
    NavHost(nav, startDestination = Routes.HOME) {
        composable(Routes.HOME) { Home(model, nav) }
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
        composable(Routes.SEARCH) {
            SearchScreen(model, onBack = { nav.popBackStack() }, onOpen = { nav.navigate(Routes.chat(it)) })
        }
    }
}

private enum class Tab { Sessions, Settings }

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Home(model: AppModel, nav: NavHostController) {
    var tab by rememberSaveable { mutableStateOf(if (model.launch.route == "settings") Tab.Settings else Tab.Sessions) }
    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        bottomBar = {
            ShortNavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainer) {
                ShortNavigationBarItem(
                    selected = tab == Tab.Sessions,
                    onClick = { tab = Tab.Sessions },
                    icon = { Icon(if (tab == Tab.Sessions) Icons.Filled.ChatBubble else Icons.Outlined.ChatBubbleOutline, null) },
                    label = { Text("Sessions") },
                )
                ShortNavigationBarItem(
                    selected = tab == Tab.Settings,
                    onClick = { tab = Tab.Settings },
                    icon = { Icon(if (tab == Tab.Settings) Icons.Filled.Settings else Icons.Outlined.Settings, null) },
                    label = { Text("Settings") },
                )
            }
        },
    ) { padding ->
        Box(Modifier.padding(bottom = padding.calculateBottomPadding())) {
            when (tab) {
                Tab.Sessions -> SessionsScreen(
                    model,
                    onOpen = { nav.navigate(Routes.chat(it)) },
                    onNew = { nav.navigate(Routes.NEW) },
                    onSearch = { nav.navigate(Routes.SEARCH) },
                )
                Tab.Settings -> SettingsScreen(model)
            }
        }
    }
}
