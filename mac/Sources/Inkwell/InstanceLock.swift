// One Inkwell per user. Two copies would both hold the dictation key and the microphone, and both
// write the same library.
//
// An exclusive flock(2) on a file in the data directory, taken before anything starts and held
// until the process ends. The kernel drops it when the process dies however it dies, so a crash
// never leaves a stale lock, and two copies launched at the same moment cannot both win. (A
// running-applications check can do neither: both copies see each other, or neither does.)
import Darwin
import Foundation

/// The held lock. Released when the process ends, or by `release`/deinit.
final class InstanceLock {
    enum Outcome {
        /// This process is the one instance.
        case acquired(InstanceLock)
        /// Another process holds it.
        case heldElsewhere
        /// The lock file could not be opened (`errno`).
        case failed(Int32)
    }

    private var descriptor: Int32

    private init(descriptor: Int32) {
        self.descriptor = descriptor
    }

    /// Takes the lock at `file`, creating it (readable by this user only) if needed. Never waits.
    static func acquire(at file: URL) -> Outcome {
        // O_EXLOCK takes the lock as part of the open, so there is no moment where the file is open
        // and unlocked; O_NONBLOCK makes a held lock fail at once (EWOULDBLOCK) instead of
        // waiting. O_CLOEXEC keeps the descriptor, and so the lock, out of any child process.
        let flags = O_RDWR | O_CREAT | O_EXLOCK | O_NONBLOCK | O_CLOEXEC
        let fd = file.withUnsafeFileSystemRepresentation { path -> Int32 in
            guard let path else { return -1 }
            return open(path, flags, 0o600)
        }
        if fd >= 0 {
            return .acquired(InstanceLock(descriptor: fd))
        }
        let error = errno
        return error == EWOULDBLOCK ? .heldElsewhere : .failed(error)
    }

    /// Lets go of the lock (tests; the app holds it until it exits).
    func release() {
        if descriptor >= 0 {
            close(descriptor)
            descriptor = -1
        }
    }

    deinit {
        release()
    }
}
