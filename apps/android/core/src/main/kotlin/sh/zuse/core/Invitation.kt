package sh.zuse.core

import java.net.URI

/** How a paired route reaches the computer. Mirrors `RouteKind` in `zuse-remote`. */
enum class RouteClass(val label: String) {
    Loopback("This computer"),
    Lan("Local network"),
    Tailnet("Tailscale"),
    Public("Direct"),
    Relay("Relay"),
    Ssh("SSH tunnel");

    companion object {
        /** The bridge reports `kind` in camelCase; unknown values fall back to the label. */
        fun from(kind: String?, label: String?): RouteClass? =
            entries.firstOrNull { it.name.equals(kind, ignoreCase = true) }
                ?: entries.firstOrNull { it.label == label }
    }
}

/**
 * A versioned `#pair=` invitation: an origin plus a private fragment. The origin is only a hint;
 * Noise authenticates the computer, so any address that reaches it is as good as another.
 */
data class Invitation(val origin: String, val fragment: String) {
    /** The full link. Contains the pairing secret: never log or display it. */
    val link: String
        get() = "$origin/$fragment"

    val host: String
        get() = URI(origin).host.orEmpty().removePrefix("[").removeSuffix("]")

    val port: Int
        get() = URI(origin).let { if (it.port != -1) it.port else if (it.scheme == "https") 443 else 80 }

    val routeClass: RouteClass
        get() = classify(origin)

    /** A link that only works from the computer itself, such as Zuse with remote access off. */
    val loopbackOnly: Boolean
        get() = routeClass == RouteClass.Loopback

    /** The same invitation reached through another address, such as an SSH tunnel. */
    fun via(newOrigin: String): Invitation = Invitation(newOrigin.trimEnd('/'), fragment)

    companion object {
        private val pattern =
            Regex("""(?:https?|wss?)://[^\s/#"'<>]+/?#pair=[A-Za-z0-9_-]{16,512}""")

        /** Finds the first invitation in free text such as `zuse pair` output. */
        fun find(text: String): Invitation? =
            pattern.findAll(text).firstNotNullOfOrNull { parse(it.value) }

        /** All invitations in the text, app links first, so callers can prefer one. */
        fun findAll(text: String): List<Invitation> =
            pattern.findAll(text).mapNotNull { parse(it.value) }.toList()

        fun parse(value: String): Invitation? {
            val trimmed = value.trim()
            val hash = trimmed.indexOf("#pair=")
            if (hash < 0) return null
            val uri = runCatching { URI(trimmed.substring(0, hash)) }.getOrNull() ?: return null
            val scheme =
                when (uri.scheme?.lowercase()) {
                    "https",
                    "wss" -> "https"
                    "http",
                    "ws" -> "http"
                    else -> return null
                }
            val host = uri.host ?: return null
            if (uri.rawUserInfo != null) return null
            val port = if (uri.port == -1) "" else ":${uri.port}"
            val origin = "$scheme://$host$port"
            return Invitation(origin, trimmed.substring(hash + 1).let { "#$it" })
        }

        fun classify(origin: String): RouteClass {
            val host =
                runCatching { URI(origin).host }.getOrNull()?.lowercase()?.removePrefix("[")?.removeSuffix("]")
                    ?: return RouteClass.Public
            if (host == "localhost" || host == "::1" || host.startsWith("127.")) return RouteClass.Loopback
            if (isTailnetHost(host)) return RouteClass.Tailnet
            if (Endpoint.isPrivateHost(host)) return RouteClass.Lan
            return RouteClass.Public
        }

        /** Tailscale and Headscale share the CGNAT range and the `fd7a:115c:a1e0::/48` prefix. */
        fun isTailnetHost(value: String): Boolean {
            val host = value.lowercase().removePrefix("[").removeSuffix("]").trimEnd('.')
            if (host.endsWith(".ts.net")) return true
            val octets = host.split('.').map { it.toIntOrNull() }
            if (octets.size == 4 && octets.all { it != null && it in 0..255 })
                return octets[0] == 100 && octets[1]!! in 64..127
            return host.startsWith("fd7a:115c:a1e0:")
        }
    }
}
