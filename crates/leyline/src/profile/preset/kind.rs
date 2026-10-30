use serde::Deserialize;

/// The kind of browser request a preset shapes headers for, named in kebab-case in profile data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum RequestKind {
    /// A request from a native client rather than a browser page.
    Native,
    /// A top-level navigation, such as typing a URL or following a link.
    Navigate,
    /// A navigation inside an iframe.
    FrameNavigate,
    /// A reload of the current page.
    Reload,
    /// A script subresource load.
    Script,
    /// An image subresource load.
    Image,
    /// A `fetch` or `XMLHttpRequest` call from page script.
    Xhr,
    /// A form submission.
    Form,
    /// A request to a different origin than the page.
    CrossOrigin,
    /// A request to the same site as the page.
    SameSite,
    /// A navigation caused by a form submission.
    FormNavigate,
}
