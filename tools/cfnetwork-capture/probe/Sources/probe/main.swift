import Foundation

// CFNetwork capture probe.
//
// Usage: cfnetwork-probe <url> [count] [delaySec]
//
// Accepts any server certificate — this rig talks ONLY to our own capture
// server (see tools/cfnetwork-capture/README.md). The ClientHello is sent
// before the server certificate is ever evaluated, so even a rejecting
// delegate would not lose the handshake capture; accepting just lets the
// HTTP/2 exchange complete for the full frame-order evidence pack.

final class TrustAllDelegate: NSObject, URLSessionDelegate {
    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust
        else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        completionHandler(.useCredential, URLCredential(trust: trust))
    }
}

func err(_ s: String) {
    FileHandle.standardError.write(Data((s + "\n").utf8))
}

let args = CommandLine.arguments
guard args.count >= 2, let url = URL(string: args[1]) else {
    err("usage: cfnetwork-probe <url> [count] [delaySec]")
    exit(64)
}
let count = args.count >= 3 ? Int(args[2]) ?? 1 : 1
let delay = args.count >= 4 ? Double(args[3]) ?? 0.5 : 0.5

let delegate = TrustAllDelegate()
let config = URLSessionConfiguration.ephemeral
config.waitsForConnectivity = false
config.timeoutIntervalForRequest = 15
let session = URLSession(configuration: config, delegate: delegate, delegateQueue: nil)

// CFN_PROBE_DUMP=1 prints the response body (used for tls.peet.ws/api/all
// fingerprint observations; propagates through `simctl spawn` via the
// SIMCTL_CHILD_ prefix).
let dump = ProcessInfo.processInfo.environment["CFN_PROBE_DUMP"] == "1"

for i in 0..<count {
    if i > 0 {
        Thread.sleep(forTimeInterval: delay)
    }
    var req = URLRequest(url: url)
    req.httpMethod = "GET"
    let sem = DispatchSemaphore(value: 0)
    let task = session.dataTask(with: req) { data, resp, error in
        if let error {
            err("run \(i + 1): ERROR \(error)")
        } else if let http = resp as? HTTPURLResponse {
            print("run \(i + 1): OK status=\(http.statusCode) bytes=\(data?.count ?? -1)")
            if dump, let data {
                FileHandle.standardOutput.write(data)
                print("")
            }
        }
        sem.signal()
    }
    task.resume()
    sem.wait()
}
session.finishTasksAndInvalidate()
exit(0)
