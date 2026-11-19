package sh.zuse.android

import java.util.Base64
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonObject
import sh.zuse.core.*

/**
 * One SSH session per computer, shared by the open chat and the background inbox refresh, so a
 * list refresh never pays for another sign-in. Sessions close after a minute without users.
 */
class SshTunnels(private val vault: Vault) {
    private class Entry(val session: SshSession) {
        val forwards = HashMap<String, Int>()
        var users = 0
        var idle: Job? = null
    }

    /** A loopback route into the computer. [close] releases it; the session may stay warm. */
    class Lease(val link: String, private val release: () -> Unit) : AutoCloseable {
        private var closed = false

        override fun close() {
            if (!closed) {
                closed = true
                release()
            }
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lock = Mutex()
    private val entries = HashMap<String, Entry>()
    private var cachedIdentity: SshIdentity? = null

    /** This phone's SSH key, created on first use and kept in the vault. */
    suspend fun identity(): SshIdentity =
        lock.withLock { identityLocked() }

    private suspend fun identityLocked(): SshIdentity {
        cachedIdentity?.let {
            return it
        }
        val saved = vault.read("ssh-identity")?.let { WireJson.parseToJsonElement(it) }
        val private = saved?.text("private")
        val public = saved?.text("public")
        val identity =
            if (private != null && public != null)
                SshIdentity(Base64.getDecoder().decode(private), public)
            else
                withContext(Dispatchers.Default) {
                        SshIdentity.generate("zuse-android@${android.os.Build.MODEL.replace(' ', '-')}")
                    }
                    .also {
                        vault.write(
                            "ssh-identity",
                            JsonObject(
                                    mapOf(
                                        "private" to Base64.getEncoder().encodeToString(it.privateKey).json(),
                                        "public" to it.publicKey.json(),
                                    )
                                )
                                .toString(),
                        )
                    }
        cachedIdentity = identity
        return identity
    }

    /** Signs in once, outside the shared pool, for pairing and installing this phone's key. */
    suspend fun signIn(target: SshTarget, password: String?): SshSession =
        SshSession.open(target, if (password == null) identity() else null, password)

    /**
     * Opens or reuses the computer's session and forwards a stable loopback port to the address in
     * its invitation, as the computer sees it.
     */
    suspend fun lease(device: SavedDevice): Lease {
        val target = requireNotNull(device.ssh) { "This computer has no SSH settings." }
        val invitation =
            device.pairingLink?.let(Invitation::parse)
                ?: throw ClientException("Pair this computer again over SSH.")
        val key = "${invitation.host}:${invitation.port}"
        val local =
            lock.withLock {
                var entry = entries[device.id]
                if (entry != null && !entry.session.alive) {
                    entry.idle?.cancel()
                    entry.session.close()
                    entries.remove(device.id)
                    entry = null
                }
                if (entry == null) {
                    entry = Entry(SshSession.open(target, identityLocked(), null))
                    entries[device.id] = entry
                }
                entry.idle?.cancel()
                entry.idle = null
                val port =
                    entry.forwards[key]
                        ?: entry.session.forward(invitation.host, invitation.port, stablePort(device.id)).also {
                            entry.forwards[key] = it
                        }
                entry.users++
                port
            }
        return Lease(invitation.via("http://127.0.0.1:$local").link) { release(device.id) }
    }

    private fun release(id: String) {
        scope.launch {
            lock.withLock {
                val entry = entries[id] ?: return@withLock
                entry.users = (entry.users - 1).coerceAtLeast(0)
                if (entry.users == 0 && entry.idle == null)
                    entry.idle = scope.launch {
                        delay(IDLE_MS)
                        lock.withLock {
                            if (entries[id] === entry && entry.users == 0) {
                                entries.remove(id)
                                entry.session.close()
                            }
                        }
                    }
            }
        }
    }

    /** Drops a computer's session at once, such as when it is forgotten. */
    fun drop(id: String) {
        scope.launch {
            lock.withLock {
                entries.remove(id)?.let {
                    it.idle?.cancel()
                    it.session.close()
                }
            }
        }
    }

    /** The network changed: sessions on the old one are dead even if JSch has not noticed. */
    fun networkChanged() {
        scope.launch {
            lock.withLock {
                entries.values.filter { it.users == 0 }.forEach {
                    it.idle?.cancel()
                    it.session.close()
                }
                entries.values.removeAll { it.users == 0 }
            }
        }
    }

    companion object {
        private const val IDLE_MS = 60_000L

        /** The same local port each time keeps the computer's saved routes from piling up. */
        fun stablePort(id: String): Int = 41_000 + Math.floorMod(id.hashCode(), 8_000)
    }
}
