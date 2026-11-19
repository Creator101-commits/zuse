@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package sh.zuse.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import sh.zuse.android.WorkspaceViewModel.ComputerState
import sh.zuse.core.*

private val connectedGreen: Color
    @Composable get() = if (isSystemInDarkTheme()) Color(0xFF3FCB7A) else Color(0xFF1F9D57)
private val pendingAmber = Color(0xFFD08C1F)

@Composable
fun stateColor(state: ComputerState): Color =
    when (state) {
        ComputerState.Connected,
        ComputerState.Reachable -> connectedGreen
        ComputerState.Connecting,
        ComputerState.Checking -> pendingAmber
        ComputerState.Unreachable -> MaterialTheme.colorScheme.error
    }

/** A status dot; live states breathe so a glance tells settled from in-progress. */
@Composable
fun StatusDot(state: ComputerState, size: Dp = 8.dp, modifier: Modifier = Modifier) {
    val color = stateColor(state)
    val pulsing = state == ComputerState.Connecting || state == ComputerState.Checking
    val halo = state == ComputerState.Connected
    val transition = rememberInfiniteTransition(label = "dot")
    val pulse by
        transition.animateFloat(
            0f,
            1f,
            infiniteRepeatable(tween(1400, easing = FastOutSlowInEasing), RepeatMode.Restart),
            label = "pulse",
        )
    Box(modifier.size(size * 2), contentAlignment = Alignment.Center) {
        if (pulsing || halo)
            Box(
                Modifier.size(size * (1f + pulse))
                    .background(color.copy(alpha = (if (halo) .22f else .35f) * (1f - pulse)), CircleShape)
            )
        Box(Modifier.size(size).background(color, CircleShape))
    }
}

/** The home header's way into Computers, with the overall state as a dot. */
@Composable
fun ComputersButton(workspace: WorkspaceViewModel, onClick: () -> Unit) {
    val state = workspace.overallState
    val description =
        when (state) {
            ComputerState.Connected -> "Computers, connected"
            ComputerState.Unreachable -> "Computers, can’t reach a computer"
            ComputerState.Connecting,
            ComputerState.Checking -> "Computers, connecting"
            ComputerState.Reachable -> "Computers, available"
            null -> "Computers"
        }
    IconButton(onClick = onClick, modifier = Modifier.size(44.dp).semantics { contentDescription = description }) {
        Box {
            Icon(Icons.Outlined.Laptop, null, Modifier.size(20.dp))
            if (state != null)
                Box(
                    Modifier.align(Alignment.TopEnd)
                        .offset(x = 3.dp, y = (-2).dp)
                        .size(9.dp)
                        .background(MaterialTheme.colorScheme.background, CircleShape)
                        .padding(1.5.dp)
                        .background(stateColor(state), CircleShape)
                )
        }
    }
}

fun ago(epochMs: Long?, now: Long = System.currentTimeMillis()): String? {
    epochMs ?: return null
    val minutes = (now - epochMs).coerceAtLeast(0) / 60_000
    return when {
        minutes < 1 -> "just now"
        minutes < 60 -> "$minutes min ago"
        minutes < 60 * 24 -> "${minutes / 60} h ago"
        else -> "${minutes / (60 * 24)} d ago"
    }
}

fun statusLine(status: WorkspaceViewModel.ComputerStatus): String =
    when (status.state) {
        ComputerState.Connected ->
            listOfNotNull("Connected", status.route, status.rttMs?.let { "$it ms" }).joinToString(" · ")
        ComputerState.Connecting -> status.route ?: "Connecting…"
        ComputerState.Checking -> "Checking…"
        ComputerState.Reachable -> "Available" + (ago(status.lastSeen)?.let { " · seen $it" } ?: "")
        ComputerState.Unreachable ->
            "Can’t reach" + (ago(status.lastSeen)?.let { " · last connected $it" } ?: "")
    }

private fun copy(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}

private fun clipboardText(context: Context): String? =
    context
        .getSystemService(ClipboardManager::class.java)
        ?.primaryClip
        ?.takeIf { it.itemCount > 0 }
        ?.getItemAt(0)
        ?.coerceToText(context)
        ?.toString()

/**
 * Everything about reaching computers in one place: what's saved and how it's connected, adding
 * one by QR code, link or SSH, and checking a connection that isn't working.
 */
@Composable
fun ComputersSheet(
    workspace: WorkspaceViewModel,
    openUri: (Uri) -> Unit,
    initialPage: String = "list",
    onCloud: () -> Unit,
    onClose: () -> Unit,
) {
    val start = if (workspace.pairingLink != null) "add" else initialPage
    var stack by rememberSaveable { mutableStateOf(listOf(start)) }
    val page = stack.last()
    fun go(next: String) {
        stack = stack + next
    }
    fun back() {
        if (stack.size > 1) stack = stack.dropLast(1) else onClose()
    }
    LaunchedEffect(Unit) { workspace.refreshComputerInboxes(quiet = true) }
    ModalBottomSheet(
        onDismissRequest = onClose,
        sheetMaxWidth = 600.dp,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
        dragHandle = { SheetHandle() },
        properties = ModalBottomSheetProperties(shouldDismissOnBackPress = stack.size == 1),
    ) {
        BackHandler(stack.size > 1) { back() }
        Column(
            Modifier.fillMaxWidth().heightIn(max = LocalConfiguration.current.screenHeightDp.dp * .92f)
        ) {
            val title =
                when {
                    page == "list" -> "Computers"
                    page == "add" -> "Add computer"
                    page == "ssh" -> "Connect with SSH"
                    page.startsWith("check:") -> "Check connection"
                    page.startsWith("detail:") ->
                        workspace.places.firstOrNull { it.first == page.removePrefix("detail:") }?.second
                            ?: "Computer"
                    else -> ""
                }
            PaneHeader(
                title,
                leading = {
                    if (stack.size > 1) SmallIcon(Icons.AutoMirrored.Outlined.ArrowBack, "Back", ::back)
                },
                actions = {
                    if (page == "list") SmallIcon(Icons.Outlined.Add, "Add computer", { go("add") })
                    SmallIcon(Icons.Outlined.Close, "Close", onClose)
                },
            )
            AnimatedContent(
                page,
                transitionSpec = {
                    val forward = stack.size > 1 && targetState == stack.last()
                    (slideInHorizontally(tween(220)) { if (forward) it / 6 else -it / 6 } + fadeIn(tween(220)))
                        .togetherWith(fadeOut(tween(120)))
                        .using(SizeTransform(clip = false))
                },
                label = "computers page",
                modifier = Modifier.weight(1f, fill = false),
            ) { current ->
                Column(
                    Modifier.fillMaxWidth()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp)
                        .padding(bottom = 24.dp)
                        .imePadding(),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    when {
                        current == "list" ->
                            ComputerList(
                                workspace,
                                onOpen = { go("detail:$it") },
                                onAdd = { go("add") },
                                onCloud = onCloud,
                                onSignIn = { workspace.signIn(openUri) },
                            )
                        current == "add" -> AddComputer(workspace, onSsh = { go("ssh") }, onDone = onClose)
                        current == "ssh" -> SshConnect(workspace, onDone = onClose)
                        current.startsWith("detail:") ->
                            ComputerDetail(
                                workspace,
                                current.removePrefix("detail:"),
                                onCheck = { go("check:$it") },
                                onShowChats = onClose,
                                onForgotten = ::back,
                            )
                        current.startsWith("check:") ->
                            workspace.devices.firstOrNull { it.id == current.removePrefix("check:") }?.let {
                                CheckConnection(workspace, it)
                            }
                    }
                }
            }
        }
    }
}

@Composable
private fun ComputerList(
    workspace: WorkspaceViewModel,
    onOpen: (String) -> Unit,
    onAdd: () -> Unit,
    onCloud: () -> Unit,
    onSignIn: () -> Unit,
) {
    val computers = workspace.places.filter { it.first != WorkspaceViewModel.CLOUD_PLACE }
    if (computers.isEmpty()) {
        Column(
            Modifier.fillMaxWidth().padding(top = 24.dp, bottom = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Box(
                Modifier.size(72.dp).background(MaterialTheme.colorScheme.surfaceContainerHigh, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Outlined.Laptop, null, Modifier.size(30.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(
                "Connect your computer",
                Modifier.padding(top = 20.dp),
                style = MaterialTheme.typography.titleLarge,
            )
            Text(
                "Your chats run on your own computer. Pair it once and reach it at home or away.",
                Modifier.padding(top = 8.dp, start = 12.dp, end = 12.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(24.dp))
            CompactAction("Add computer", onClick = onAdd)
        }
    } else {
        Group {
            computers.forEachIndexed { index, (key, name) ->
                if (index > 0) InsetDivider()
                val status = workspace.computerStatus(key)
                val device = workspace.deviceFor(key)
                ComputerRow(
                    name,
                    status,
                    tag =
                        when {
                            device?.ssh != null -> "SSH"
                            key.startsWith("computer:") -> "Account"
                            else -> null
                        },
                    current = workspace.currentPlace == key,
                ) {
                    onOpen(key)
                }
            }
        }
    }
    Group {
        if (workspace.signedIn)
            NavRow(Icons.Outlined.CloudQueue, "Zuse Cloud", "Hosted chats that run without your computer", onClick = onCloud)
        else
            NavRow(
                Icons.Outlined.AccountCircle,
                "Sign in to Zuse",
                "Reach computers linked to your account, and Zuse Cloud",
                onClick = onSignIn,
            )
    }
    if (computers.isNotEmpty())
        OutlinedButton(
            onClick = onAdd,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
            shape = RoundedCornerShape(12.dp),
        ) {
            Icon(Icons.Outlined.Add, null, Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text("Add computer")
        }
}

@Composable
internal fun ComputerRow(
    name: String,
    status: WorkspaceViewModel.ComputerStatus,
    tag: String?,
    current: Boolean,
    onClick: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth()
            .clickable(onClickLabel = "Open $name", onClick = onClick)
            .semantics(mergeDescendants = true) {}
            .heightIn(min = 68.dp)
            .padding(start = 16.dp, end = 12.dp, top = 12.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            Modifier.size(40.dp).background(MaterialTheme.colorScheme.surfaceContainerHigh, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Outlined.Laptop, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            StatusDot(status.state, 7.dp, Modifier.align(Alignment.BottomEnd).offset(x = 5.dp, y = 5.dp))
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    name,
                    Modifier.weight(1f, fill = false),
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (tag != null) Tag(tag)
                if (current) Tag("Open")
            }
            Text(
                statusLine(status),
                style = MaterialTheme.typography.bodySmall,
                color =
                    if (status.state == ComputerState.Unreachable) MaterialTheme.colorScheme.error
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(Icons.Outlined.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun Tag(text: String) {
    Text(
        text,
        Modifier.background(MaterialTheme.colorScheme.surfaceContainerHighest, RoundedCornerShape(6.dp))
            .padding(horizontal = 6.dp, vertical = 1.dp),
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun ComputerDetail(
    workspace: WorkspaceViewModel,
    key: String,
    onCheck: (String) -> Unit,
    onShowChats: () -> Unit,
    onForgotten: () -> Unit,
) {
    val name = workspace.places.firstOrNull { it.first == key }?.second ?: "Computer"
    val device = workspace.deviceFor(key)
    val status = workspace.computerStatus(key)
    var renaming by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }
    val context = LocalContext.current
    Column(
        Modifier.fillMaxWidth().padding(top = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            Modifier.size(64.dp).background(MaterialTheme.colorScheme.surfaceContainerHigh, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Outlined.Laptop, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Row(
            Modifier.background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(50))
                .padding(start = 4.dp, end = 12.dp, top = 2.dp, bottom = 2.dp)
                .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            StatusDot(status.state, 7.dp)
            Text(statusLine(status), style = MaterialTheme.typography.labelLarge)
        }
    }
    status.hint?.let { hint ->
        Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(14.dp)) {
            Row(
                Modifier.fillMaxWidth().padding(14.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Icon(Icons.Outlined.Lightbulb, null, Modifier.size(18.dp), tint = pendingAmber)
                Text(hint, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
    Group {
        InfoRow("Route", status.route?.let { r -> status.rttMs?.let { "$r · $it ms" } ?: r } ?: "—")
        InsetDivider()
        InfoRow(
            "Connected through",
            when {
                device?.ssh != null ->
                    "SSH · ${device.ssh?.display}" + if (device.tunnel) " (tunnel)" else ""
                device?.pairingLink != null -> "Pairing link"
                device != null -> "Zuse serve address"
                else -> "Zuse account"
            },
        )
        device?.pairingLink?.let(Invitation::parse)?.takeIf { !it.loopbackOnly }?.let { invitation ->
            InsetDivider()
            InfoRow("Paired at", "${invitation.routeClass.label} · ${invitation.host}")
        }
        device?.ssh?.hostKey?.let {
            InsetDivider()
            InfoRow("Host key", it, mono = true) { copy(context, "Host key", it) }
        }
        ago(status.lastSeen)?.let {
            InsetDivider()
            InfoRow("Last connected", it)
        }
    }
    Group {
        NavRow(Icons.Outlined.ChatBubbleOutline, "Show its chats", chevron = false) {
            workspace.showPlace(key)
            onShowChats()
        }
        InsetDivider()
        val target = workspace.targetFor(key)
        NavRow(
            Icons.Outlined.Refresh,
            if (workspace.currentPlace == key) "Reconnect" else "Try again",
            chevron = false,
        ) {
            if (workspace.currentPlace == key) workspace.reconnect()
            else if (target != null) workspace.refreshComputerInboxes()
        }
        if (device != null) {
            InsetDivider()
            NavRow(Icons.Outlined.NetworkCheck, "Check connection", "Find out what’s in the way") { onCheck(device.id) }
            InsetDivider()
            NavRow(Icons.Outlined.DriveFileRenameOutline, "Rename", chevron = false) { renaming = true }
        }
    }
    if (device != null)
        Group {
            NavRow(Icons.Outlined.DeleteOutline, "Forget this computer", destructive = true, chevron = false) {
                forgetting = true
            }
        }
    if (renaming && device != null) {
        var text by remember { mutableStateOf(device.label) }
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Rename computer") },
            text = {
                OutlinedTextField(text, { text = it }, singleLine = true, label = { Text("Name") }, shape = RoundedCornerShape(12.dp))
            },
            confirmButton = {
                TextButton(
                    enabled = text.isNotBlank(),
                    onClick = {
                        workspace.renameDevice(device, text)
                        renaming = false
                    },
                ) {
                    Text("Save")
                }
            },
            dismissButton = { TextButton(onClick = { renaming = false }) { Text("Cancel") } },
        )
    }
    if (forgetting && device != null)
        AlertDialog(
            onDismissRequest = { forgetting = false },
            title = { Text("Forget ${device.label}?") },
            text = {
                Text(
                    "This phone stops connecting to it. To keep it from accepting this phone, revoke it on the computer under Settings → Devices."
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        forgetting = false
                        workspace.forgetDevice(device)
                        onForgotten()
                    }
                ) {
                    Text("Forget", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { forgetting = false }) { Text("Cancel") } },
        )
}

@Composable
private fun AddComputer(workspace: WorkspaceViewModel, onSsh: () -> Unit, onDone: () -> Unit) {
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    var link by rememberSaveable { mutableStateOf(workspace.pairingLink.orEmpty()) }
    var name by rememberSaveable { mutableStateOf("") }
    var failure by remember { mutableStateOf<String?>(null) }
    var submitted by remember { mutableStateOf(false) }
    fun submit(value: String) {
        val text = value.trim()
        if (text.isEmpty() || workspace.busy) return
        failure = null
        submitted = true
        workspace.error = null
        workspace.pair(text, name, null) {
            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
            onDone()
        }
    }
    // Pairing reports through the shared error; show it here, once, under the field.
    LaunchedEffect(workspace.error, workspace.busy) {
        if (submitted && !workspace.busy && workspace.error != null) {
            failure = workspace.error
            workspace.error = null
            submitted = false
        }
    }
    // A link opened from elsewhere is filled in, never submitted: pairing is the user's call.
    val scanner =
        rememberLauncherForActivityResult(ScanContract()) { result ->
            result.contents?.let {
                link = it
                submit(it)
            }
        }
    Surface(
        onClick = {
            scanner.launch(
                ScanOptions()
                    .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    .setPrompt("Scan the code shown by Zuse on your computer")
                    .setBeepEnabled(false)
                    .setOrientationLocked(false)
            )
        },
        shape = RoundedCornerShape(24.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth().height(196.dp).semantics { contentDescription = "Scan QR code" },
    ) {
        Column(
            Modifier.fillMaxSize().padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            val corner = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .5f)
            Box(
                Modifier.size(76.dp).drawBehind {
                    val l = size.minDimension * .28f
                    val w = 2.5.dp.toPx()
                    val r = size.width
                    listOf(
                        androidx.compose.ui.geometry.Offset(0f, 0f) to androidx.compose.ui.geometry.Offset(l, 0f),
                        androidx.compose.ui.geometry.Offset(0f, 0f) to androidx.compose.ui.geometry.Offset(0f, l),
                        androidx.compose.ui.geometry.Offset(r, 0f) to androidx.compose.ui.geometry.Offset(r - l, 0f),
                        androidx.compose.ui.geometry.Offset(r, 0f) to androidx.compose.ui.geometry.Offset(r, l),
                        androidx.compose.ui.geometry.Offset(0f, r) to androidx.compose.ui.geometry.Offset(l, r),
                        androidx.compose.ui.geometry.Offset(0f, r) to androidx.compose.ui.geometry.Offset(0f, r - l),
                        androidx.compose.ui.geometry.Offset(r, r) to androidx.compose.ui.geometry.Offset(r - l, r),
                        androidx.compose.ui.geometry.Offset(r, r) to androidx.compose.ui.geometry.Offset(r, r - l),
                    )
                        .forEach { (a, b) ->
                            drawLine(corner, a, b, w, androidx.compose.ui.graphics.StrokeCap.Round)
                        }
                },
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Outlined.QrCodeScanner, null, Modifier.size(32.dp))
            }
            Text("Scan QR code", Modifier.padding(top = 14.dp), style = MaterialTheme.typography.titleMedium)
            Text(
                "On your computer: Zuse → Settings → Devices",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Or paste a link", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        OutlinedTextField(
            link,
            {
                link = it
                failure = null
            },
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Pairing link" },
            placeholder = { Text("http://…/#pair=…") },
            singleLine = true,
            isError = failure != null,
            shape = RoundedCornerShape(14.dp),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
            visualTransformation = if (link.length > 48) PasswordVisualTransformation('•') else androidx.compose.ui.text.input.VisualTransformation.None,
            trailingIcon = {
                if (link.isEmpty())
                    TextButton(onClick = { clipboardText(context)?.let { link = it.trim() } }) { Text("Paste") }
                else SmallIcon(Icons.Outlined.Close, "Clear link", { link = "" })
            },
        )
        AnimatedVisibility(link.isNotBlank()) {
            OutlinedTextField(
                name,
                { name = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text("Name (optional)") },
                singleLine = true,
                shape = RoundedCornerShape(14.dp),
            )
        }
        failure?.let {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Icon(Icons.Outlined.ErrorOutline, null, Modifier.size(16.dp).padding(top = 2.dp), tint = MaterialTheme.colorScheme.error)
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
        }
        Button(
            onClick = { submit(link) },
            enabled = link.isNotBlank() && !workspace.busy,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
            shape = RoundedCornerShape(12.dp),
            colors = ButtonDefaults.buttonColors(containerColor = brandFill, contentColor = Color(0xFF17220B)),
        ) {
            if (workspace.busy) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = Color(0xFF17220B))
                Spacer(Modifier.width(10.dp))
                Text("Connecting…")
            } else Text("Connect")
        }
    }
    Group {
        NavRow(
            Icons.Outlined.Terminal,
            "Connect with SSH",
            "Sign in to a computer or server and let Zuse set it up",
            onClick = onSsh,
        )
    }
    Accordion("Where do I find the link?") {
        HelpText("On a computer with the Zuse app: Settings → Devices → Connect a device. Show the QR code or copy the link.")
        HelpText("On a server or a computer without a screen, start Zuse and create a link:")
        CommandChip("zuse serve --lan")
        CommandChip("zuse pair --app")
    }
    Accordion("Reach it from anywhere") {
        HelpText(
            "Turn on Tailscale on this phone and the computer (Headscale works too), and turn on Direct connections on the computer. Zuse picks the fastest route, at home or away, and keeps everything end-to-end encrypted."
        )
        HelpText("Or link the computer to your Zuse account, then sign in here:")
        CommandChip("zuse link")
    }
}

@Composable
private fun SshConnect(workspace: WorkspaceViewModel, onDone: () -> Unit) {
    val context = LocalContext.current
    val progress = workspace.sshProgress
    var address by rememberSaveable { mutableStateOf(progress?.target?.display.orEmpty()) }
    var name by rememberSaveable { mutableStateOf("") }
    var usePassword by rememberSaveable { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var alwaysTunnel by rememberSaveable { mutableStateOf(false) }
    var keepRunning by rememberSaveable { mutableStateOf(true) }
    val target = SshTarget.parse(address)
    LaunchedEffect(Unit) { workspace.loadSshKey() }
    DisposableEffect(Unit) { onDispose { workspace.resetSsh() } }
    val running = progress?.running != null
    fun run(start: Boolean = false) {
        val t = target ?: return
        workspace.pairOverSsh(
            t.copy(password = usePassword),
            name,
            password.takeIf { usePassword && it.isNotEmpty() },
            start = start,
            keepRunning = keepRunning,
            alwaysTunnel = alwaysTunnel,
            done = onDone,
        )
    }
    if (progress == null || (progress.failed == null && progress.running == null && !progress.finished)) {
        HelpText("Zuse signs in, finds or starts Zuse there, and pairs this phone. Works over your network, Tailscale and Headscale.")
        OutlinedTextField(
            address,
            { address = it.trim() },
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = "SSH address" },
            label = { Text("Computer") },
            placeholder = { Text("you@computer.local") },
            singleLine = true,
            isError = address.isNotBlank() && target == null,
            supportingText = {
                if (address.isNotBlank() && target == null) Text("Use user@host, optionally with :port")
            },
            shape = RoundedCornerShape(14.dp),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
        )
        OutlinedTextField(
            name,
            { name = it },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("Name (optional)") },
            singleLine = true,
            shape = RoundedCornerShape(14.dp),
        )
        Segmented(listOf("This phone’s key", "Password"), if (usePassword) 1 else 0) { usePassword = it == 1 }
        if (usePassword)
            OutlinedTextField(
                password,
                { password = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Password") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                supportingText = { Text("Used once to add this phone’s key. Not saved.") },
                shape = RoundedCornerShape(14.dp),
            )
        else PublicKeyCard(workspace.sshPublicKey) { copy(context, "SSH key", it) }
        Accordion("Options") {
            SwitchRow("Always connect through SSH", "Otherwise Zuse connects directly when it can, and uses SSH when it can’t.", alwaysTunnel) {
                alwaysTunnel = it
            }
        }
        Button(
            onClick = { run() },
            enabled = target != null && (!usePassword || password.isNotEmpty()),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
            shape = RoundedCornerShape(12.dp),
            colors = ButtonDefaults.buttonColors(containerColor = brandFill, contentColor = Color(0xFF17220B)),
        ) {
            Text("Connect")
        }
        return
    }
    Text(progress.target.display, style = MaterialTheme.typography.titleMedium, fontFamily = FontFamily.Monospace)
    Group {
        WorkspaceViewModel.SshStep.entries.forEachIndexed { index, step ->
            if (index > 0) InsetDivider()
            val detail =
                when (step) {
                    WorkspaceViewModel.SshStep.SignIn -> progress.hostKey?.let { "Host key ${it.take(19)}…" }
                    WorkspaceViewModel.SshStep.FindZuse -> progress.version
                    else -> null
                }
            StepRow(
                step.title,
                when {
                    step in progress.done -> true
                    step == progress.failed -> false
                    else -> null
                },
                running = step == progress.running,
                detail = detail,
            )
        }
    }
    progress.message?.let { message ->
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Outlined.ErrorOutline, null, Modifier.size(16.dp).padding(top = 2.dp), tint = MaterialTheme.colorScheme.error)
            Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
        }
    }
    when {
        progress.needsKey -> {
            PublicKeyCard(workspace.sshPublicKey) { copy(context, "SSH key", it) }
            HelpText("Or sign in with your password once. Zuse adds this phone’s key so you won’t need it again.")
            OutlinedTextField(
                password,
                { password = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Password") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                shape = RoundedCornerShape(14.dp),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { run() }, shape = RoundedCornerShape(12.dp)) { Text("I added the key") }
                Button(
                    onClick = {
                        usePassword = true
                        run()
                    },
                    enabled = password.isNotEmpty(),
                    shape = RoundedCornerShape(12.dp),
                ) {
                    Text("Use password")
                }
            }
        }
        progress.needsStart -> {
            SwitchRow("Keep it running", "Start Zuse again when the computer restarts.", keepRunning) { keepRunning = it }
            CompactAction("Start Zuse") { run(start = true) }
        }
        progress.failed != null ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { workspace.cancelSsh() }, shape = RoundedCornerShape(12.dp)) { Text("Edit") }
                CompactAction("Try again") { run() }
            }
        running ->
            TextButton(onClick = { workspace.cancelSsh() }) { Text("Cancel") }
    }
}

@Composable
private fun PublicKeyCard(key: String?, onCopy: (String) -> Unit) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(14.dp)) {
        Column(Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Key, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(
                    "This phone’s SSH key",
                    Modifier.weight(1f).padding(start = 8.dp),
                    style = MaterialTheme.typography.labelLarge,
                )
                if (key != null)
                    TextButton(onClick = { onCopy(key) }, contentPadding = PaddingValues(horizontal = 10.dp)) {
                        Icon(Icons.Outlined.ContentCopy, null, Modifier.size(15.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Copy")
                    }
            }
            Text(
                key ?: "Creating key…",
                style = MaterialTheme.typography.bodySmall.copy(fontSize = 11.sp),
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "Add it to ~/.ssh/authorized_keys on the computer, or sign in with a password once.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun CheckConnection(workspace: WorkspaceViewModel, device: SavedDevice) {
    LaunchedEffect(device.id) { workspace.checkConnection(device) }
    HelpText("Checking ${device.label} one step at a time.")
    Group {
        workspace.checkSteps.forEachIndexed { index, step ->
            if (index > 0) InsetDivider()
            StepRow(step.title, step.ok, detail = step.detail, failedDetailIsError = true)
        }
        if (workspace.checking) {
            if (workspace.checkSteps.isNotEmpty()) InsetDivider()
            StepRow("Checking…", null, running = true)
        }
    }
    if (workspace.checkCanStart && !workspace.checking) {
        var keepRunning by rememberSaveable { mutableStateOf(true) }
        SwitchRow("Keep it running", "Start Zuse again when the computer restarts.", keepRunning) { keepRunning = it }
        CompactAction("Start Zuse") { workspace.startZuse(device, keepRunning) }
    } else if (!workspace.checking) CompactAction("Check again") { workspace.checkConnection(device) }
}

@Composable
private fun StepRow(
    title: String,
    ok: Boolean?,
    running: Boolean = false,
    detail: String? = null,
    failedDetailIsError: Boolean = false,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
            when {
                running -> CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                ok == true -> Icon(Icons.Outlined.CheckCircle, "Done", Modifier.size(20.dp), tint = connectedGreen)
                ok == false -> Icon(Icons.Outlined.ErrorOutline, "Failed", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.error)
                else ->
                    Box(
                        Modifier.size(14.dp)
                            .border(1.5.dp, MaterialTheme.colorScheme.outlineVariant, CircleShape)
                    )
            }
        }
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyMedium,
                color =
                    if (ok == null && !running) MaterialTheme.colorScheme.onSurfaceVariant
                    else MaterialTheme.colorScheme.onSurface,
            )
            if (detail != null)
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    color =
                        if (ok == false && failedDetailIsError) MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
    }
}

// ---- Small building blocks shared by these pages ----

@Composable
fun Group(content: @Composable ColumnScope.() -> Unit) {
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth(), content = content)
    }
}

@Composable
fun InsetDivider() {
    HorizontalDivider(
        Modifier.padding(start = 16.dp),
        thickness = 0.5.dp,
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .6f),
    )
}

@Composable
fun NavRow(
    icon: ImageVector,
    title: String,
    subtitle: String? = null,
    destructive: Boolean = false,
    chevron: Boolean = true,
    onClick: () -> Unit,
) {
    val tint = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        Modifier.fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Icon(icon, null, Modifier.size(20.dp), tint = tint)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyLarge,
                color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
            )
            if (subtitle != null)
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
        if (chevron)
            Icon(Icons.Outlined.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun InfoRow(label: String, value: String, mono: Boolean = false, onClick: (() -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(onClickLabel = "Copy", onClick = onClick) else Modifier)
            .semantics(mergeDescendants = true) {}
            .heightIn(min = 48.dp)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            value,
            Modifier.weight(1f),
            style = if (mono) MaterialTheme.typography.bodySmall else MaterialTheme.typography.bodyMedium,
            fontFamily = if (mono) FontFamily.Monospace else null,
            textAlign = TextAlign.End,
            maxLines = 2,
            overflow = TextOverflow.MiddleEllipsis,
        )
    }
}

@Composable
fun Accordion(title: String, content: @Composable ColumnScope.() -> Unit) {
    var open by rememberSaveable(title) { mutableStateOf(false) }
    val rotation by animateFloatAsState(if (open) 90f else 0f, tween(180), label = "chevron")
    Column(Modifier.fillMaxWidth().animateContentSize(tween(200))) {
        Row(
            Modifier.fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .clickable { open = !open }
                .semantics {
                    role = Role.Button
                    stateDescription = if (open) "Expanded" else "Collapsed"
                }
                .heightIn(min = 44.dp)
                .padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(Icons.Outlined.ChevronRight, null, Modifier.size(18.dp).rotate(rotation), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (open)
            Column(
                Modifier.padding(start = 30.dp, end = 4.dp, bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
                content = content,
            )
    }
}

@Composable
fun HelpText(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** A shell command with a one-tap copy. */
@Composable
fun CommandChip(command: String) {
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            kotlinx.coroutines.delay(1_500)
            copied = false
        }
    }
    Surface(
        onClick = {
            copy(context, "Command", command)
            copied = true
        },
        shape = RoundedCornerShape(10.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Copy $command" },
    ) {
        Row(Modifier.padding(start = 12.dp, end = 8.dp).heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                command,
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
            Icon(
                if (copied) Icons.Outlined.Check else Icons.Outlined.ContentCopy,
                null,
                Modifier.size(15.dp),
                tint = if (copied) connectedGreen else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun Segmented(labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Row(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(12.dp))
            .padding(3.dp)
    ) {
        labels.forEachIndexed { index, label ->
            val active = index == selected
            Box(
                Modifier.weight(1f)
                    .height(38.dp)
                    .clip(RoundedCornerShape(9.dp))
                    .background(if (active) MaterialTheme.colorScheme.surface else Color.Transparent)
                    .clickable { onSelect(index) }
                    .semantics {
                        role = Role.Tab
                        this.selected = active
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    label,
                    style = MaterialTheme.typography.labelLarge,
                    color = if (active) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun SwitchRow(title: String, detail: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth()
            .clickable(role = Role.Switch) { onChange(!checked) }
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked, null)
    }
}
