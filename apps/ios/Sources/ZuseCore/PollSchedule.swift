import Foundation

/// Background refresh cadence for cloud state: quick while something is changing, slower while idle.
public enum PollSchedule {
  /// Seconds between refreshes while a workspace is starting, pausing, or otherwise changing.
  public static let cloudTransition: Double = 4

  /// Doubling delay for the nth consecutive quiet or failed poll, capped.
  public static func backoff(attempt: Int, initial: Double, maximum: Double) -> Double {
    min(maximum, initial * pow(2, Double(min(max(0, attempt), 16))))
  }

  /// Sign-in stays responsive while the user is likely finishing in the browser, then eases off.
  public static func loginDelay(elapsed: Double) -> Double {
    elapsed < 60 ? 2 : 10
  }
}
