package sh.zuse.core

import kotlin.test.*

class PollScheduleTests {
    @Test
    fun `quiet polls double up to the cap`() {
        assertEquals(30_000L, PollSchedule.backoff(0, 30_000, 60_000))
        assertEquals(60_000L, PollSchedule.backoff(1, 30_000, 60_000))
        assertEquals(60_000L, PollSchedule.backoff(1_000, 30_000, 60_000))
        assertEquals(30_000L, PollSchedule.backoff(-1, 30_000, 60_000))
        assertEquals(40_000L, PollSchedule.backoff(2, 10_000, 60_000))
    }

    @Test
    fun `sign-in stays responsive for a minute then eases off`() {
        assertEquals(2_000L, PollSchedule.loginDelay(0))
        assertEquals(2_000L, PollSchedule.loginDelay(59_999))
        assertEquals(10_000L, PollSchedule.loginDelay(60_000))
        assertEquals(10_000L, PollSchedule.loginDelay(600_000))
    }
}
