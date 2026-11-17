import Testing

@testable import ZuseCore

@Test func pollBackoffDoublesUpToTheCap() {
  #expect(PollSchedule.backoff(attempt: 0, initial: 3, maximum: 30) == 3)
  #expect(PollSchedule.backoff(attempt: 1, initial: 3, maximum: 30) == 6)
  #expect(PollSchedule.backoff(attempt: 3, initial: 3, maximum: 30) == 24)
  #expect(PollSchedule.backoff(attempt: 4, initial: 3, maximum: 30) == 30)
  #expect(PollSchedule.backoff(attempt: 1_000, initial: 3, maximum: 30) == 30)
  #expect(PollSchedule.backoff(attempt: -1, initial: 3, maximum: 30) == 3)
}

@Test func signInPollingStaysResponsiveForAMinuteThenEasesOff() {
  #expect(PollSchedule.loginDelay(elapsed: 0) == 2)
  #expect(PollSchedule.loginDelay(elapsed: 59.9) == 2)
  #expect(PollSchedule.loginDelay(elapsed: 60) == 10)
  #expect(PollSchedule.loginDelay(elapsed: 600) == 10)
}
