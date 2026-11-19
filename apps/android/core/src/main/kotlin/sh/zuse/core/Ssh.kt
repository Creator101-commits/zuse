package sh.zuse.core

import com.jcraft.jsch.HostKey
import com.jcraft.jsch.HostKeyRepository
import com.jcraft.jsch.JSch
import com.jcraft.jsch.JSchException
import com.jcraft.jsch.KeyPair
import com.jcraft.jsch.Session
import com.jcraft.jsch.UIKeyboardInteractive
import com.jcraft.jsch.UserInfo
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.security.MessageDigest
import java.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable

/** Where and how to sign in over SSH. Contains no secrets; passwords and keys live in the vault. */
@Serializable
data class SshTarget(
    val host: String,
    val user: String,
    val port: Int = 22,
    /** `SHA256:` fingerprint trusted on first use. A different key is refused. */
    val hostKey: String? = null,
    val password: Boolean = false,
) {
    val display: String
        get() = if (port == 22) "$user@$host" else "$user@$host:$port"

    companion object {
        /** Accepts `user@host`, `user@host:2222` and `ssh://user@host:2222`. */
        fun parse(value: String, fallbackUser: String = ""): SshTarget? {
            val text = value.trim().removePrefix("ssh://").trimEnd('/')
            if (text.isEmpty() || text.any { it.isWhitespace() }) return null
            val at = text.lastIndexOf('@')
            val user = if (at > 0) text.substring(0, at) else fallbackUser
            val rest = if (at >= 0) text.substring(at + 1) else text
            val (host, port) =
                if (rest.startsWith("[")) {
                    val end = rest.indexOf(']')
                    if (end < 0) return null
                    rest.substring(1, end) to
                        rest.substring(end + 1).removePrefix(":").ifEmpty { "22" }
                } else if (rest.count { it == ':' } == 1)
                    rest.substringBefore(':') to rest.substringAfter(':')
                else rest to "22"
            val number = port.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
            if (host.isBlank() || user.isBlank()) return null
            return SshTarget(host, user, number)
        }
    }
}

/** An SSH key made on this phone. The private half never leaves the vault. */
class SshIdentity(val privateKey: ByteArray, val publicKey: String) {
    companion object {
        fun generate(comment: String): SshIdentity {
            val keys = KeyPair.genKeyPair(JSch(), KeyPair.ECDSA, 256)
            try {
                val private = ByteArrayOutputStream().also { keys.writePrivateKey(it) }
                val public = ByteArrayOutputStream().also { keys.writePublicKey(it, comment) }
                return SshIdentity(
                    private.toByteArray(),
                    public.toString(Charsets.UTF_8).trim(),
                )
            } finally {
                keys.dispose()
            }
        }
    }
}

class SshException(message: String, val kind: Kind = Kind.Other) : ClientException(message) {
    enum class Kind {
        Unreachable,
        Auth,
        HostKeyChanged,
        NoZuse,
        NotRunning,
        Other,
    }
}

data class SshResult(val exit: Int, val output: String)

/**
 * One signed-in SSH connection. Blocking JSch calls run on the IO dispatcher; [close] is safe from
 * any thread and ends every forward.
 */
class SshSession private constructor(private val session: Session, val hostKey: String) :
    Closeable {
    val alive: Boolean
        get() = session.isConnected

    /** Runs [command] with the common install locations of `zuse` on PATH. */
    suspend fun exec(command: String, timeoutMs: Long = 20_000): SshResult =
        withContext(Dispatchers.IO) {
            val channel = session.openChannel("exec") as com.jcraft.jsch.ChannelExec
            val output = ByteArrayOutputStream()
            try {
                channel.setCommand("$PATH_PREFIX$command")
                channel.setInputStream(null)
                channel.setErrStream(output, true)
                channel.outputStream.close()
                val stream = channel.inputStream
                channel.connect(10_000)
                val deadline = System.currentTimeMillis() + timeoutMs
                val buffer = ByteArray(8192)
                while (true) {
                    ensureActive()
                    while (stream.available() > 0) {
                        val read = stream.read(buffer)
                        if (read < 0) break
                        output.write(buffer, 0, read)
                        if (output.size() > MAX_OUTPUT) throw SshException("The command printed too much output.")
                    }
                    if (channel.isClosed && stream.available() == 0) break
                    if (System.currentTimeMillis() > deadline)
                        throw SshException("The computer took too long to answer.")
                    Thread.sleep(40)
                }
                SshResult(channel.exitStatus, output.toString(Charsets.UTF_8))
            } finally {
                channel.disconnect()
            }
        }

    /**
     * Listens on loopback and forwards to [host]:[port] as the remote computer sees it. Tries
     * [preferredPort] first so saved routes stay stable between launches.
     */
    suspend fun forward(host: String, port: Int, preferredPort: Int = 0): Int =
        withContext(Dispatchers.IO) {
            try {
                session.setPortForwardingL("127.0.0.1", preferredPort, host, port)
            } catch (_: JSchException) {
                session.setPortForwardingL("127.0.0.1", 0, host, port)
            }
        }

    override fun close() {
        runCatching { session.disconnect() }
    }

    companion object {
        private const val MAX_OUTPUT = 512 * 1024

        /**
         * Non-interactive shells skip profile files, so add where installers put `zuse`. English
         * output keeps the messages below recognizable on any locale.
         */
        const val PATH_PREFIX =
            "PATH=\"\$HOME/.local/bin:\$HOME/.zuse/bin:\$HOME/bin:/usr/local/bin:/opt/homebrew/bin:" +
                "/Applications/Zuse.app/Contents/MacOS:\$PATH\"; ZUSE_LANGUAGE=en; export PATH ZUSE_LANGUAGE; "

        fun fingerprint(key: ByteArray): String =
            "SHA256:" +
                Base64.getEncoder()
                    .withoutPadding()
                    .encodeToString(MessageDigest.getInstance("SHA-256").digest(key))

        suspend fun open(
            target: SshTarget,
            identity: SshIdentity?,
            password: String?,
        ): SshSession =
            withContext(Dispatchers.IO) {
                val jsch = JSch()
                identity?.let {
                    jsch.addIdentity("zuse", it.privateKey, it.publicKey.toByteArray(), null)
                }
                var seen: String? = null
                jsch.hostKeyRepository = TrustOnFirstUse(target.hostKey) { seen = it }
                val session = jsch.getSession(target.user, target.host, target.port)
                session.setConfig("StrictHostKeyChecking", "yes")
                session.setConfig(
                    "PreferredAuthentications",
                    if (password != null) "password,keyboard-interactive,publickey" else "publickey",
                )
                if (password != null) {
                    session.setPassword(password)
                    session.userInfo = PasswordOnly(password)
                }
                session.setServerAliveInterval(15_000)
                session.setServerAliveCountMax(3)
                try {
                    session.connect(12_000)
                } catch (failure: JSchException) {
                    session.disconnect()
                    throw explain(failure, target, seen)
                }
                SshSession(session, requireNotNull(seen) { "Missing host key." })
            }

        private fun explain(failure: JSchException, target: SshTarget, seen: String?): SshException {
            val message = failure.message.orEmpty()
            val cause = failure.cause
            return when {
                message.contains("HostKey has been changed", true) ||
                    message.contains("reject HostKey", true) ->
                    SshException(
                        "${target.host} presented a different SSH host key than before. " +
                            "If you reinstalled it, forget this computer and add it again.",
                        SshException.Kind.HostKeyChanged,
                    )
                message.contains("Auth fail", true) || message.contains("Auth cancel", true) ->
                    SshException(
                        if (target.password) "The password was not accepted."
                        else "${target.user}@${target.host} did not accept this phone's key. Add the key to ~/.ssh/authorized_keys, or sign in with a password once.",
                        SshException.Kind.Auth,
                    )
                cause is java.net.UnknownHostException ->
                    SshException(
                        "Can’t find ${target.host}. Check the name, or turn on Tailscale if it is a tailnet name.",
                        SshException.Kind.Unreachable,
                    )
                cause is java.net.ConnectException ||
                    cause is java.net.NoRouteToHostException ||
                    cause is java.net.SocketTimeoutException ||
                    message.contains("timeout", true) ->
                    SshException(
                        "Can’t reach ${target.host}:${target.port}. Make sure it is on and SSH is enabled.",
                        SshException.Kind.Unreachable,
                    )
                else -> SshException("SSH failed: ${message.ifBlank { "unknown error" }}")
            }
        }
    }

    private class PasswordOnly(private val password: String) : UserInfo, UIKeyboardInteractive {
        override fun getPassphrase(): String? = null

        override fun getPassword(): String = password

        override fun promptPassword(message: String?) = true

        override fun promptPassphrase(message: String?) = false

        override fun promptYesNo(message: String?) = false

        override fun showMessage(message: String?) {}

        override fun promptKeyboardInteractive(
            destination: String?,
            name: String?,
            instruction: String?,
            prompt: Array<out String>,
            echo: BooleanArray,
        ): Array<String> = Array(prompt.size) { password }
    }

    /** Accepts any key the first time and only the recorded fingerprint after that. */
    private class TrustOnFirstUse(private val expected: String?, private val record: (String) -> Unit) :
        HostKeyRepository {
        override fun check(host: String?, key: ByteArray): Int {
            val print = fingerprint(key)
            record(print)
            return when (expected) {
                null,
                print -> HostKeyRepository.OK
                else -> HostKeyRepository.CHANGED
            }
        }

        override fun add(hostkey: HostKey?, ui: UserInfo?) {}

        override fun remove(host: String?, type: String?) {}

        override fun remove(host: String?, type: String?, key: ByteArray?) {}

        override fun getKnownHostsRepositoryID() = "zuse"

        override fun getHostKey(): Array<HostKey> = emptyArray()

        override fun getHostKey(host: String?, type: String?): Array<HostKey> = emptyArray()
    }
}

/** What `zuse` reports over SSH, and the invitation it creates for this phone. */
object ZuseOverSsh {
    data class Pairing(val invitation: Invitation, val version: String?)

    private fun quote(value: String) = "'" + value.replace("'", "'\\''") + "'"

    suspend fun version(session: SshSession): String? {
        val found = session.exec("command -v zuse >/dev/null 2>&1 && zuse --version", 10_000)
        return found.output.lineSequence().firstOrNull { it.isNotBlank() }?.trim()?.takeIf {
            found.exit == 0
        }
    }

    /** The computer's own name, such as "Ana’s MacBook Pro", for labeling it on this phone. */
    suspend fun computerName(session: SshSession): String? =
        runCatching {
                session
                    .exec("scutil --get ComputerName 2>/dev/null || hostname -s 2>/dev/null || hostname", 10_000)
                    .output
                    .lineSequence()
                    .map { it.trim() }
                    .firstOrNull { it.isNotEmpty() && it.length <= 80 }
            }
            .getOrNull()

    /**
     * Asks the running engine for an invitation. Older CLIs lack `--json`, so plain output is
     * searched as well; the app link is printed after the browser link.
     */
    suspend fun pair(session: SshSession, label: String): Invitation {
        val name = quote(label.take(60))
        val result =
            session.exec("zuse pair --json --label $name 2>&1 || zuse pair --label $name 2>&1", 20_000)
        val output = result.output
        jsonInvitation(output)?.let { return it }
        Invitation.findAll(output).lastOrNull()?.let { return it }
        val lower = output.lowercase()
        throw when {
            result.exit == 127 ->
                SshException("Zuse isn’t installed on this computer.", SshException.Kind.NoZuse)
            "no zuse server is running" in lower || "not running" in lower ->
                SshException("Zuse isn’t running on this computer.", SshException.Kind.NotRunning)
            // The engine is up but nothing accepts connections yet; `zuse serve` adds that.
            "could not create a connection link" in lower ->
                SshException(
                    "Zuse is running but not accepting connections yet.",
                    SshException.Kind.NotRunning,
                )
            else ->
                SshException(
                    output.lineSequence().lastOrNull { it.isNotBlank() }?.trim()?.take(200)
                        ?: "Zuse could not create a pairing link."
                )
        }
    }

    internal fun jsonInvitation(output: String): Invitation? {
        val line = output.lineSequence().map { it.trim() }.firstOrNull { it.startsWith("{") } ?: return null
        val value = runCatching { WireJson.parseToJsonElement(line) }.getOrNull() ?: return null
        return (value.text("app") ?: value.text("browser"))?.let(Invitation::parse)
    }

    /** Starts `zuse serve` in the background; it keeps running after this session ends. */
    suspend fun start(session: SshSession, persistent: Boolean) {
        val command =
            if (persistent) "zuse daemon install --serve 2>&1"
            else
                // The log holds the owner link, so only this user may read it.
                "umask 077; nohup zuse serve >\"\$HOME/.zuse-serve.log\" 2>&1 </dev/null & sleep 1; echo started"
        val result = session.exec(command, 30_000)
        if (result.exit == 127) throw SshException("Zuse isn’t installed on this computer.", SshException.Kind.NoZuse)
        if (result.exit != 0)
            throw SshException(
                result.output.lineSequence().lastOrNull { it.isNotBlank() }?.trim()?.take(200)
                    ?: "Zuse did not start."
            )
    }

    /** Appends this phone's public key, so later sign-ins need no password. */
    suspend fun authorize(session: SshSession, publicKey: String) {
        require(publicKey.startsWith("ssh-") && '\n' !in publicKey && '\'' !in publicKey)
        val result =
            session.exec(
                "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && " +
                    "(grep -qxF ${quote(publicKey)} ~/.ssh/authorized_keys || " +
                    "printf '%s\\n' ${quote(publicKey)} >> ~/.ssh/authorized_keys)",
                10_000,
            )
        if (result.exit != 0) throw SshException("Could not add this phone’s key on the computer.")
    }
}
