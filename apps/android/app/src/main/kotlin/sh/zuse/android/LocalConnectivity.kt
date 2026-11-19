package sh.zuse.android

import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.X509Certificate
import javax.net.ssl.*
import okhttp3.OkHttpClient
import sh.zuse.core.RpcClient

/** Pins are SHA-256 of the certificate DER, matching zuse serve's discovery contract. */
fun pinnedHttp(pin: String?): OkHttpClient {
    if (pin == null) return RpcClient.defaultHttp()
    require(Regex("[A-Za-z0-9_-]{43}").matches(pin)) { "Invalid computer certificate pin." }
    fun matches(cert: X509Certificate) =
        MessageDigest.isEqual(
            MessageDigest.getInstance("SHA-256").digest(cert.encoded).base64Url().toByteArray(),
            pin.toByteArray(),
        )
    val trust =
        object : X509TrustManager {
            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()

            override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
                throw java.security.cert.CertificateException("Client certificates unsupported")
            }

            override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
                if (chain.isEmpty() || !matches(chain[0]))
                    throw java.security.cert.CertificateException(
                        "The computer certificate changed. Pair again."
                    )
            }
        }
    val tls = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trust), SecureRandom()) }
    return RpcClient.defaultHttp()
        .newBuilder()
        .sslSocketFactory(tls.socketFactory, trust)
        .hostnameVerifier { _, session ->
            (session.peerCertificates.firstOrNull() as? X509Certificate)?.let(::matches) == true
        }
        .build()
}
