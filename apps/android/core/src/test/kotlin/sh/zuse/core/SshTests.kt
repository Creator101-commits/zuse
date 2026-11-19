package sh.zuse.core

import java.net.ServerSocket
import java.nio.file.Files
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import org.apache.sshd.common.config.keys.PublicKeyEntry
import org.apache.sshd.server.SshServer
import org.apache.sshd.server.auth.pubkey.PublickeyAuthenticator
import org.apache.sshd.server.forward.AcceptAllForwardingFilter
import org.apache.sshd.server.keyprovider.SimpleGeneratorHostKeyProvider

class InvitationTests {
    private val fragment = "#pair=eyJ2IjoxLCJpZCI6ImFiYyJ9AAAAAAAA"

    @Test
    fun findsTheAppLinkInPlainPairOutput() {
        val output =
            """
            Browser link: https://192.168.1.4:27659/$fragment
            App link: http://192.168.1.4:27660/$fragment
            Anyone with this link can use this computer.
            """
                .trimIndent()
        val links = Invitation.findAll(output)
        assertEquals(2, links.size)
        assertEquals("http://192.168.1.4:27660", links.last().origin)
        assertEquals(RouteClass.Lan, links.last().routeClass)
        assertEquals(fragment, links.last().fragment)
    }

    @Test
    fun prefersJsonOutput() {
        val json = """{"app":"http://127.0.0.1:27656/$fragment","browser":null}"""
        val invitation = assertNotNull(ZuseOverSsh.jsonInvitation("noise\n$json\n"))
        assertTrue(invitation.loopbackOnly)
        assertEquals(27656, invitation.port)
        val tunnelled = invitation.via("http://127.0.0.1:41000")
        assertEquals("http://127.0.0.1:41000/$fragment", tunnelled.link)
    }

    @Test
    fun classifiesTailnetsIncludingHeadscale() {
        assertEquals(RouteClass.Tailnet, Invitation.classify("http://100.101.4.2:27660"))
        assertEquals(RouteClass.Tailnet, Invitation.classify("https://mac.tail1234.ts.net"))
        assertEquals(RouteClass.Tailnet, Invitation.classify("http://[fd7a:115c:a1e0::12]:27660"))
        assertEquals(RouteClass.Lan, Invitation.classify("http://10.0.0.8:27660"))
        assertEquals(RouteClass.Loopback, Invitation.classify("http://localhost:27656"))
        assertEquals(RouteClass.Public, Invitation.classify("https://relay.example.com"))
    }

    @Test
    fun rejectsLinksWithCredentialsOrOtherSchemes() {
        assertNull(Invitation.parse("http://user:pw@10.0.0.2/$fragment"))
        assertNull(Invitation.parse("ftp://10.0.0.2/$fragment"))
        assertNull(Invitation.parse("http://10.0.0.2/#token=abc"))
    }

    @Test
    fun parsesSshTargets() {
        assertEquals(SshTarget("mac.local", "ana", 22), SshTarget.parse("ana@mac.local"))
        assertEquals(SshTarget("10.0.0.2", "ana", 2222), SshTarget.parse("ssh://ana@10.0.0.2:2222"))
        assertEquals(SshTarget("fd7a::1", "ana", 22), SshTarget.parse("ana@[fd7a::1]"))
        assertEquals(SshTarget("box", "me", 22), SshTarget.parse("box", "me"))
        assertNull(SshTarget.parse("box"))
        assertNull(SshTarget.parse("ana@host:99999"))
    }
}

class SshTests {
    private val identity = SshIdentity.generate("zuse-test")
    private lateinit var server: SshServer

    @BeforeTest
    fun start() {
        val publicKey = PublicKeyEntry.parsePublicKeyEntry(identity.publicKey).resolvePublicKey(null, null, null)
        server =
            SshServer.setUpDefaultServer().apply {
                host = "127.0.0.1"
                port = 0
                keyPairProvider =
                    SimpleGeneratorHostKeyProvider(Files.createTempFile("host", ".key").also { Files.delete(it) })
                publickeyAuthenticator = PublickeyAuthenticator { _, key, _ -> key == publicKey }
                passwordAuthenticator =
                    org.apache.sshd.server.auth.password.PasswordAuthenticator { user, password, _ ->
                        user == "ana" && password == "secret"
                    }
                // Like sshd, run each command through a shell.
                commandFactory =
                    org.apache.sshd.server.command.CommandFactory { channel, command ->
                        org.apache.sshd.server.shell.ProcessShellFactory(command, "/bin/sh", "-c", command)
                            .createShell(channel)
                    }
                forwardingFilter = AcceptAllForwardingFilter.INSTANCE
                start()
            }
    }

    @AfterTest fun stop() = server.stop(true)

    private fun target(hostKey: String? = null, password: Boolean = false) =
        SshTarget("127.0.0.1", "ana", server.port, hostKey, password)

    @Test
    fun runsCommandsAndForwardsWithThePhoneKey() = runBlocking {
        SshSession.open(target(), identity, null).use { session ->
            assertTrue(session.hostKey.startsWith("SHA256:"))
            val result = session.exec("echo hello; exit 3")
            assertEquals("hello", result.output.trim())
            assertEquals(3, result.exit)
            ServerSocket(0, 1, java.net.InetAddress.getLoopbackAddress()).use { echo ->
                val local = session.forward("127.0.0.1", echo.localPort)
                val accepted = Thread {
                    echo.accept().use { it.getOutputStream().write("pong".toByteArray()) }
                }.apply { start() }
                java.net.Socket("127.0.0.1", local).use {
                    assertEquals("pong", it.getInputStream().readBytes().toString(Charsets.UTF_8))
                }
                accepted.join(5_000)
            }
        }
    }

    @Test
    fun signsInWithAPasswordAndRefusesAChangedHostKey() = runBlocking {
        val first = SshSession.open(target(password = true), null, "secret").use { it.hostKey }
        SshSession.open(target(first), identity, null).use { assertEquals(first, it.hostKey) }
        val changed =
            assertFailsWith<SshException> {
                SshSession.open(target("SHA256:somethingelse"), identity, null)
            }
        assertEquals(SshException.Kind.HostKeyChanged, changed.kind)
        val refused = assertFailsWith<SshException> { SshSession.open(target(password = true), null, "nope") }
        assertEquals(SshException.Kind.Auth, refused.kind)
    }

    @Test
    fun reportsAnUnreachableHost() = runBlocking {
        val port = ServerSocket(0).use { it.localPort }
        val failure =
            assertFailsWith<SshException> {
                SshSession.open(SshTarget("127.0.0.1", "ana", port), identity, null)
            }
        assertEquals(SshException.Kind.Unreachable, failure.kind)
    }
}
