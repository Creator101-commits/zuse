package sh.zuse.android

import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * What this phone's network looks like right now. Tailscale and Headscale both run as the system
 * VPN, so [vpn] is the best available signal that tailnet addresses can be reached.
 */
class PhoneNetwork {
    var online by mutableStateOf(true)
        private set

    var wifi by mutableStateOf(false)
        private set

    var vpn by mutableStateOf(false)
        private set

    fun refresh(connectivity: ConnectivityManager) {
        val active = connectivity.activeNetwork?.let(connectivity::getNetworkCapabilities)
        online = active?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
        @Suppress("DEPRECATION")
        val all = connectivity.allNetworks.mapNotNull(connectivity::getNetworkCapabilities)
        vpn = all.any { it.hasTransport(NetworkCapabilities.TRANSPORT_VPN) }
        wifi =
            all.any {
                it.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                    it.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
            }
    }
}
