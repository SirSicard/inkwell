// The Apple engines' own log. It never carries what was said: no transcript, partial or prompt.
import os

enum Log {
    static let engine = Logger(subsystem: "com.inkwell.app", category: "apple-engines")
}
