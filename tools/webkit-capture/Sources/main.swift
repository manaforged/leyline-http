import AppKit
import Foundation
import WebKit

/// Offscreen WKWebView GET. Same WebKit TLS as Safari on this Mac.
/// Does not activate Safari.app.
final class Box: NSObject, WKNavigationDelegate {
    let webView: WKWebView

    override init() {
        let config = WKWebViewConfiguration()
        webView = WKWebView(frame: CGRect(x: 0, y: 0, width: 8, height: 8), configuration: config)
        super.init()
        webView.navigationDelegate = self
    }

    func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        webView.evaluateJavaScript("document.body ? document.body.innerText : ''") { result, error in
            if let text = result as? String, text.contains("{") {
                FileHandle.standardOutput.write(Data(text.utf8))
                FileHandle.standardOutput.write(Data("\n".utf8))
                exit(0)
            }
            let why = error?.localizedDescription ?? "empty body"
            FileHandle.standardError.write(Data("webkit-probe: \(why)\n".utf8))
            exit(1)
        }
    }

    func webView(_: WKWebView, didFail _: WKNavigation!, withError error: Error) {
        FileHandle.standardError.write(Data("webkit-probe: \(error.localizedDescription)\n".utf8))
        exit(1)
    }

    func webView(_: WKWebView, didFailProvisionalNavigation _: WKNavigation!, withError error: Error) {
        FileHandle.standardError.write(Data("webkit-probe: \(error.localizedDescription)\n".utf8))
        exit(1)
    }
}

guard CommandLine.arguments.count >= 2, let url = URL(string: CommandLine.arguments[1]) else {
    FileHandle.standardError.write(Data("usage: webkit-probe <url>\n".utf8))
    exit(64)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let box = Box()
box.webView.load(URLRequest(url: url))
DispatchQueue.main.asyncAfter(deadline: .now() + 35) {
    FileHandle.standardError.write(Data("webkit-probe: timeout\n".utf8))
    exit(1)
}
app.run()
