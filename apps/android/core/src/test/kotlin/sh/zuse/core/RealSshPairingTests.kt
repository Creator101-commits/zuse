package sh.zuse.core

import java.nio.file.Files
import java.nio.file.Path
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import org.apache.sshd.common.config.keys.PublicKeyEntry
import org.apache.sshd.server.SshServer
import org.apache.sshd.server.auth.pubkey.PublickeyAuthenticator
import org.apache.sshd.server.command.CommandFactory
import org.apache.sshd.server.forward.AcceptAllForwardingFilter
import org.apache.sshd.server.keyprovider.SimpleGeneratorHostKeyProvider
import org.apache.sshd.server.shell.ProcessShellFactory

/**
 * Pairs over SSH with the actual `zuse` command and an isolated engine, then connects through the
 * tunnel. Run by `scripts/test-ssh-pairing.py`; skipped without its environment.
 */
class RealSshPairingTests {
    private val binary = System.getenv("ZUSE_SSH_E2E_BIN")
    private val ipcPort = System.getenv("ZUSE_SSH_E2E_IPC_PORT")
    private val identity = SshIdentity.generate("zuse-e2e")
    private lateinit var server: SshServer
    private lateinit var home: Path

    private fun serve(ipc: String) {
        home = Files.createTempDirectory("zuse-ssh-home")
        Files.createDirectories(home.resolve(".local/bin"))
        Files.createSymbolicLink(home.resolve(".local/bin/zuse"), Path.of(requireNotNull(binary)))
        val key = PublicKeyEntry.parsePublicKeyEntry(identity.publicKey).resolvePublicKey(null, null, null)
        server =
            SshServer.setUpDefaultServer().apply {
                host = "127.0.0.1"
                port = 0
                keyPairProvider =
                    SimpleGeneratorHostKeyProvider(Files.createTempFile("host", ".key").also { Files.delete(it) })
                publickeyAuthenticator = PublickeyAuthenticator { _, offered, _ -> offered == key }
                // A login shell for a user whose engine listens on this agent port.
                commandFactory = CommandFactory { channel, command ->
                    val scoped = "HOME='$home'; ZUSE_IPC_PORT=$ipc; export HOME ZUSE_IPC_PORT; $command"
                    ProcessShellFactory(scoped, "/bin/sh", "-c", scoped).createShell(channel)
                }
                forwardingFilter = AcceptAllForwardingFilter.INSTANCE
                start()
            }
    }

    @AfterTest
    fun stop() {
        if (::server.isInitialized) server.stop(true)
    }

    private fun target() = SshTarget("127.0.0.1", System.getProperty("user.name"), server.port)

    @Test
    fun pairsOverSshAndConnectsThroughTheTunnel(): Unit = runBlocking {
        if (binary == null || ipcPort == null) return@runBlocking
        serve(ipcPort)
        SshSession.open(target(), identity, null).use { session ->
            assertNotNull(ZuseOverSsh.version(session), "zuse --version over SSH")
            val invitation = ZuseOverSsh.pair(session, "SSH acceptance")
            // This engine serves loopback only, the case SSH exists for.
            assertTrue(invitation.loopbackOnly, invitation.origin)
            val local = session.forward(invitation.host, invitation.port)
            val client = RpcClient("ws://127.0.0.1:$local/rpc", pairingLink = invitation.via("http://127.0.0.1:$local").link)
            try {
                client.connect()
                assertFalse(client.call("EngineInfo").text("deviceId").isNullOrBlank())
                assertNotNull(CanonicalWorkspace(client).projects())
            } finally {
                client.close()
            }
        }
    }

    @Test
    fun reportsAnEngineThatIsNotRunning(): Unit = runBlocking {
        if (binary == null) return@runBlocking
        val closed = java.net.ServerSocket(0).use { it.localPort }
        serve(closed.toString())
        SshSession.open(target(), identity, null).use { session ->
            val failure = assertFailsWith<SshException> { ZuseOverSsh.pair(session, "SSH acceptance") }
            assertEquals(SshException.Kind.NotRunning, failure.kind, failure.message)
        }
    }
}
