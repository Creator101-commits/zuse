package sh.zuse.core

/** Background refresh cadence for cloud state: quick while something is changing, slower while idle. */
object PollSchedule {
    /** Doubling delay for the nth consecutive quiet poll, capped. */
    fun backoff(attempt: Int, initialMillis: Long, maxMillis: Long): Long =
        (initialMillis shl attempt.coerceIn(0, 16)).coerceAtMost(maxMillis)

    /** Sign-in stays responsive while the user is likely finishing in the browser, then eases off. */
    fun loginDelay(elapsedMillis: Long): Long = if (elapsedMillis < 60_000) 2_000 else 10_000
}
