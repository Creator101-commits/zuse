@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package sh.zuse.android

import android.net.Uri
import androidx.compose.foundation.*
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import sh.zuse.core.*

/** 28dp visible controls with compact tonal surfaces and a 44dp interactive row. */
@Composable
fun CompactField(
    label: String,
    value: String,
    onChange: (String) -> Unit,
    secret: Boolean = false,
    enabled: Boolean = true,
    placeholder: String? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = placeholder?.let { text -> { Text(text) } },
        enabled = enabled,
        singleLine = true,
        modifier =
            Modifier.fillMaxWidth().padding(vertical = 4.dp).semantics {
                contentDescription = label
            },
        shape = RoundedCornerShape(12.dp),
        keyboardOptions =
            androidx.compose.foundation.text.KeyboardOptions(
                keyboardType =
                    if (secret) androidx.compose.ui.text.input.KeyboardType.Password
                    else androidx.compose.ui.text.input.KeyboardType.Text,
                autoCorrectEnabled = false,
            ),
        visualTransformation =
            if (secret) PasswordVisualTransformation() else VisualTransformation.None,
    )
}

@Composable
fun CompactAction(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.heightIn(min = 48.dp),
        contentPadding = PaddingValues(horizontal = 24.dp, vertical = 12.dp),
        shape = RoundedCornerShape(12.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = brandFill,
                contentColor = Color(0xFF17220B),
            ),
    ) {
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
fun SettingsSheet(
    workspace: WorkspaceViewModel,
    openUri: (Uri) -> Unit,
    onCloud: () -> Unit,
    onComputers: () -> Unit,
    onClose: () -> Unit,
    initialPage: String = "home",
) {
    var page by rememberSaveable { mutableStateOf(initialPage) }
    ModalBottomSheet(
        onDismissRequest = onClose,
        sheetMaxWidth = 600.dp,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
        dragHandle = { SheetHandle() },
        properties =
            ModalBottomSheetProperties(shouldDismissOnBackPress = page == "home" || page == initialPage),
    ) {
        androidx.activity.compose.BackHandler(page != "home" && page != initialPage) { page = "home" }
        Column(
            Modifier.fillMaxWidth()
                .heightIn(
                    max =
                        androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp *
                            .92f
                )
        ) {
            PaneHeader(
                when (page) {
                    "home" -> "Settings"
                    "account" -> "Account"
                    else -> "Help"
                },
                leading = {
                    if (page != "home")
                        SmallIcon(Icons.AutoMirrored.Outlined.ArrowBack, "Back to settings", { page = "home" })
                },
                actions = { SmallIcon(Icons.Outlined.Close, "Close settings", onClose) },
            )
            androidx.compose.animation.AnimatedContent(
                page,
                transitionSpec = {
                    val forward = targetState != "home"
                    (androidx.compose.animation.slideInHorizontally(androidx.compose.animation.core.tween(220)) {
                            if (forward) it / 6 else -it / 6
                        } + androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(220)))
                        .togetherWith(androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(120)))
                },
                label = "settings page",
                modifier = Modifier.weight(1f, fill = false),
            ) { current ->
                Column(
                    Modifier.fillMaxWidth()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp)
                        .padding(bottom = 24.dp)
                        .imePadding(),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    workspace.error?.let {
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                    when (current) {
                        "home" -> SettingsHome(workspace, openUri, onCloud, onComputers, onClose) { page = it }
                        "account" -> AccountPage(workspace, openUri, onCloud)
                        else -> HelpPage(openUri, onComputers)
                    }
                }
            }
        }
    }
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text,
        Modifier.padding(start = 4.dp, top = 8.dp).semantics { heading() },
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun Avatar(workspace: WorkspaceViewModel, size: androidx.compose.ui.unit.Dp) {
    val account = workspace.account
    val initials =
        (account.displayName ?: account.email)
            ?.split(' ', '.', '@')
            ?.filter { it.isNotBlank() }
            ?.take(2)
            ?.joinToString("") { it.first().uppercase() }
    Box(
        Modifier.size(size)
            .background(
                if (workspace.signedIn && initials != null) brandFill
                else MaterialTheme.colorScheme.surfaceContainerHighest,
                androidx.compose.foundation.shape.CircleShape,
            ),
        contentAlignment = Alignment.Center,
    ) {
        if (workspace.signedIn && initials != null)
            Text(
                initials,
                style = if (size > 56.dp) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.titleMedium,
                color = Color(0xFF17220B),
            )
        else
            Icon(
                Icons.Outlined.Person,
                null,
                Modifier.size(size / 2),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
    }
}

@Composable
private fun SettingsHome(
    workspace: WorkspaceViewModel,
    openUri: (Uri) -> Unit,
    onCloud: () -> Unit,
    onComputers: () -> Unit,
    onClose: () -> Unit,
    go: (String) -> Unit,
) {
    val account = workspace.account
    // Profile
    Surface(
        onClick = { go("account") },
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Avatar(workspace, 48.dp)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    if (workspace.signedIn) account.displayName ?: account.email ?: "Zuse account"
                    else "Not signed in",
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    if (workspace.signedIn) account.email?.takeIf { account.displayName != null } ?: "Zuse Cloud and linked computers"
                    else "Sign in for Zuse Cloud and computers linked to your account",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (!workspace.signedIn)
                FilledTonalButton(onClick = { workspace.signIn(openUri) }, shape = RoundedCornerShape(12.dp)) {
                    Text("Sign in")
                }
            else
                Icon(Icons.Outlined.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }

    SectionHeader("Computers")
    val computers = workspace.places.filter { it.first != WorkspaceViewModel.CLOUD_PLACE }
    Group {
        computers.take(3).forEachIndexed { index, (key, name) ->
            if (index > 0) InsetDivider()
            val device = workspace.deviceFor(key)
            ComputerRow(
                name,
                workspace.computerStatus(key),
                tag = if (device?.ssh != null) "SSH" else if (key.startsWith("computer:")) "Account" else null,
                current = workspace.currentPlace == key,
                onClick = onComputers,
            )
        }
        if (computers.isNotEmpty()) InsetDivider()
        NavRow(
            if (computers.isEmpty()) Icons.Outlined.Add else Icons.Outlined.Devices,
            if (computers.isEmpty()) "Add computer" else "Manage computers",
            if (computers.isEmpty()) "Scan a code, paste a link or use SSH"
            else if (computers.size > 3) "${computers.size} computers" else null,
            onClick = onComputers,
        )
    }

    SectionHeader("About")
    Group {
        NavRow(Icons.AutoMirrored.Outlined.HelpOutline, "Help", "Connecting computers, Tailscale and SSH") { go("help") }
        InsetDivider()
        NavRow(Icons.Outlined.MailOutline, "Contact support", chevron = false) {
            openUri(Uri.parse("mailto:hi@zuse.sh"))
        }
        InsetDivider()
        Row(
            Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Icon(Icons.Outlined.Info, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text("Version", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
            Text(
                BuildConfig.VERSION_NAME,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun AccountPage(workspace: WorkspaceViewModel, openUri: (Uri) -> Unit, onCloud: () -> Unit) {
    val account = workspace.account
    var confirmSignOut by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Avatar(workspace, 72.dp)
        Text(
            if (workspace.signedIn) account.displayName ?: "Zuse account" else "Sign in to Zuse",
            Modifier.padding(top = 8.dp),
            style = MaterialTheme.typography.titleLarge,
        )
        Text(
            if (workspace.signedIn) account.email ?: "Signed in"
            else "One account for Zuse Cloud and every computer you link.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
    }
    if (!workspace.signedIn) {
        Group {
            SettingsRow(Icons.Outlined.CloudQueue, "Cloud chats", "Start chats on hosted computers, even when yours is off.", chevron = false)
            InsetDivider()
            SettingsRow(Icons.Outlined.Laptop, "Linked computers", "Reach computers linked with zuse link from anywhere.", chevron = false)
        }
        Button(
            onClick = { workspace.signIn(openUri) },
            enabled = !workspace.cloudLoading,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
            shape = RoundedCornerShape(12.dp),
            colors = ButtonDefaults.buttonColors(containerColor = brandFill, contentColor = Color(0xFF17220B)),
        ) {
            Text("Sign in")
        }
        return
    }
    Group {
        NavRow(Icons.Outlined.CloudQueue, "Zuse Cloud and agents", "Workspaces and agent connections", onClick = onCloud)
        InsetDivider()
        NavRow(
            Icons.Outlined.Refresh,
            if (workspace.cloudLoading) "Refreshing…" else "Refresh account",
            "Linked computers and cloud chats",
            chevron = false,
        ) {
            if (!workspace.cloudLoading) workspace.refreshCloud()
        }
    }
    Group {
        NavRow(Icons.Outlined.Logout, "Sign out", destructive = true, chevron = false) {
            confirmSignOut = true
        }
    }
    if (confirmSignOut)
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            title = { Text("Sign out?") },
            text = { Text("Cloud chats and linked computers disappear from this phone until you sign in again. Paired computers stay.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmSignOut = false
                    workspace.signOut()
                }) {
                    Text("Sign out", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") } },
        )
}

@Composable
private fun HelpPage(openUri: (Uri) -> Unit, onComputers: () -> Unit) {
    Group {
        SettingsRow(Icons.Outlined.Laptop, "Your computer", "Chats run on the computer you connect. Projects are its repositories.", chevron = false)
        InsetDivider()
        SettingsRow(
            Icons.Outlined.Wifi,
            "At home or away",
            "Zuse picks the fastest route: your Wi-Fi, Tailscale or Headscale, or SSH. Everything stays end-to-end encrypted.",
            chevron = false,
        )
        InsetDivider()
        SettingsRow(Icons.Outlined.CloudQueue, "Cloud chats", "A cloud workspace is a hosted computer running a repository and agent.", chevron = false)
    }
    SectionHeader("On a server or a computer without a screen")
    CommandChip("zuse serve --lan")
    CommandChip("zuse pair --app")
    SectionHeader("Reach it through your account")
    CommandChip("zuse link")
    Spacer(Modifier.height(4.dp))
    CompactAction("Add a computer", onClick = onComputers)
    TextButton(onClick = { openUri(Uri.parse("mailto:hi@zuse.sh")) }) { Text("Contact support") }
}

@Composable
fun SheetHandle() {
    Box(Modifier.fillMaxWidth().height(20.dp), contentAlignment = Alignment.Center) {
        Box(
            Modifier.size(28.dp, 3.dp)
                .background(
                    MaterialTheme.colorScheme.onSurface.copy(alpha = .18f),
                    RoundedCornerShape(2.dp),
                )
        )
    }
}


@Composable
private fun SettingsRow(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    subtitle: String? = null,
    chevron: Boolean = true,
    onClick: (() -> Unit)? = null,
) {
    Row(
        Modifier.fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .heightIn(min = 68.dp)
            .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(icon, null, Modifier.size(19.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            if (subtitle != null)
                Text(
                    subtitle,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = if (onClick == null) 3 else 1,
                    overflow = TextOverflow.Ellipsis,
                )
        }
        if (chevron && onClick != null)
            Icon(
                Icons.Outlined.ChevronRight,
                null,
                Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
    }
}


@Composable
fun DiffSheet(workspace: WorkspaceViewModel, onClose: () -> Unit) {
    var browse by rememberSaveable { mutableStateOf(false) }
    if (browse) { FileBrowserSheet(workspace) { browse = false }; return }
    var scope by rememberSaveable { mutableStateOf("unstaged") }
    var file by remember { mutableStateOf<String?>(null) }
    file?.let { path -> FileDialog(workspace, path) { file = null } }
    LaunchedEffect(scope) { workspace.loadDiff(scope) }
    DisposableEffect(Unit) { onDispose { workspace.closeDiff() } }
    ModalBottomSheet(
        onDismissRequest = onClose,
        sheetMaxWidth = 620.dp,
        dragHandle = { SheetHandle() },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(Modifier.fillMaxHeight(.92f).fillMaxWidth()) {
            PaneHeader(
                "Changes",
                actions = {
                    SmallIcon(Icons.Outlined.Folder, "Browse all files", { browse = true })
                    SmallIcon(
                        Icons.Outlined.Refresh,
                        "Refresh changes",
                        { workspace.loadDiff(scope) },
                    )
                    SmallIcon(Icons.Outlined.Close, "Close changes", onClose)
                },
            )
            val scopes = listOf("unstaged", "staged", "branch")
            CompactTabs(listOf("Working tree", "Staged", "Branch"), scopes.indexOf(scope)) {
                scope = scopes[it]
            }
            workspace.error?.let {
                Text(
                    it,
                    Modifier.padding(horizontal = 16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            if (workspace.diffLoading) LinearProgressIndicator(Modifier.fillMaxWidth())
            LazyColumn(
                Modifier.weight(1f).fillMaxWidth(),
                contentPadding = PaddingValues(vertical = 12.dp),
            ) {
                workspace.patches.forEach { patch ->
                    item(key = "path:${patch.path}") {
                        Text(
                            patch.path,
                            Modifier.clickable { file = patch.path }
                                .fillMaxWidth()
                                .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                                .padding(12.dp),
                            style = MaterialTheme.typography.labelMedium,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                    if (patch.error != null || patch.mode == "binary" || patch.truncated)
                        item(key = "note:${patch.path}") {
                            Text(
                                patch.error
                                    ?: if (patch.mode == "binary") "Binary file changed"
                                    else "Patch truncated by server",
                                Modifier.padding(12.dp),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    itemsIndexed(
                        patch.lines,
                        key = { index, _ -> "${patch.path}:$index" },
                        contentType = { _, _ -> "diff" },
                    ) { _, line ->
                        val color =
                            when (line.kind) {
                                DiffLine.Kind.ADDITION -> Color(0x24269A3B)
                                DiffLine.Kind.DELETION -> Color(0x24D93F4C)
                                else -> Color.Transparent
                            }
                        Row(
                            Modifier.fillMaxWidth()
                                .background(color)
                                .padding(horizontal = 8.dp, vertical = 2.dp)
                        ) {
                            Text(
                                line.oldLine?.toString().orEmpty(),
                                Modifier.width(34.dp),
                                style =
                                    MaterialTheme.typography.labelSmall.copy(
                                        fontSize = 10.sp,
                                        lineHeight = 20.sp,
                                    ),
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                line.newLine?.toString().orEmpty(),
                                Modifier.width(34.dp),
                                style =
                                    MaterialTheme.typography.labelSmall.copy(
                                        fontSize = 10.sp,
                                        lineHeight = 20.sp,
                                    ),
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            androidx.compose.foundation.text.selection.SelectionContainer {
                                Text(
                                    line.text,
                                    fontFamily = FontFamily.Monospace,
                                    style =
                                        MaterialTheme.typography.bodySmall.copy(
                                            fontSize = 11.sp,
                                            lineHeight = 20.sp,
                                        ),
                                    color =
                                        if (line.kind == DiffLine.Kind.HEADER) Color(0xFFA63AA5)
                                        else MaterialTheme.colorScheme.onSurface,
                                    modifier = Modifier.horizontalScroll(rememberScrollState()),
                                )
                            }
                        }
                    }
                }
                if (!workspace.diffLoading && workspace.patches.isEmpty())
                    item {
                        Text(
                            "No changes",
                            Modifier.padding(24.dp),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
            }
        }
    }
}

@Composable
internal fun FileDialog(workspace: WorkspaceViewModel, path: String, onClose: () -> Unit) {
    val openedSource = remember { workspace.source to workspace.selectedSession?.id }
    LaunchedEffect(workspace.source, workspace.selectedSession?.id) {
        if (openedSource != (workspace.source to workspace.selectedSession?.id)) onClose()
    }
    var lines by remember(path) { mutableStateOf<List<String>?>(null) }
    var failure by remember(path) { mutableStateOf<String?>(null) }
    LaunchedEffect(path) {
        try {
            lines = workspace.readFile(path)
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            failure = error.message
        }
    }
    androidx.compose.ui.window.Dialog(
        onDismissRequest = onClose,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            Modifier.fillMaxSize().padding(16.dp).widthIn(max = 1100.dp),
            shape = RoundedCornerShape(12.dp),
        ) {
            Column {
                PaneHeader(
                    path,
                    actions = { SmallIcon(Icons.Outlined.Close, "Close file", onClose) },
                )
                if (failure != null)
                    Text(
                        failure.orEmpty(),
                        Modifier.padding(16.dp),
                        color = MaterialTheme.colorScheme.error,
                    )
                else if (lines == null) CircularProgressIndicator(Modifier.padding(16.dp))
                else
                    LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(12.dp)) {
                        itemsIndexed(lines.orEmpty()) { index, text ->
                            Row(Modifier.padding(vertical = 2.dp)) {
                                Text(
                                    "${index + 1}",
                                    Modifier.width(40.dp),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                androidx.compose.foundation.text.selection.SelectionContainer {
                                    Text(
                                        text.ifEmpty { " " },
                                        fontFamily = FontFamily.Monospace,
                                        style = MaterialTheme.typography.bodySmall,
                                        modifier = Modifier.horizontalScroll(rememberScrollState()),
                                    )
                                }
                            }
                        }
                    }
            }
        }
    }
}

@Composable
fun CompactTabs(labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        labels.forEachIndexed { index, label ->
            Surface(
                shape = RoundedCornerShape(8.dp),
                color =
                    if (selected == index) MaterialTheme.colorScheme.secondaryContainer
                    else Color.Transparent,
                modifier =
                    Modifier.height(28.dp)
                        .clickable { onSelect(index) }
                        .semantics {
                            role = Role.Tab
                            this.selected = index == selected
                        },
            ) {
                Box(Modifier.padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                    Text(
                        label,
                        style = MaterialTheme.typography.labelMedium,
                        color =
                            if (selected == index) MaterialTheme.colorScheme.onSurface
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
