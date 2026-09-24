use super::mut_only::MutOnly;
use super::{
    ClientHello, GetSessionPendingError, PrivateKeyMethod, PrivateKeyMethodError, SelectCertError,
    Ssl, SslAlert, SslContextBuilder, SslRef, SslSession, SslSignatureAlgorithm, SslVerifyError,
    SslVerifyMode,
};
use crate::ex_data::Index;
use std::convert::identity;
use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::{ready, Context, Poll, Waker};

pub type BoxSelectCertFuture = ExDataFuture<Result<BoxSelectCertFinish, AsyncSelectCertError>>;

pub type BoxSelectCertFinish = Box<dyn FnOnce(ClientHello<'_>) -> Result<(), AsyncSelectCertError>>;

pub type BoxPrivateKeyMethodFuture =
    ExDataFuture<Result<BoxPrivateKeyMethodFinish, AsyncPrivateKeyMethodError>>;

pub type BoxPrivateKeyMethodFinish =
    Box<dyn FnOnce(&mut SslRef, &mut [u8]) -> Result<usize, AsyncPrivateKeyMethodError>>;

pub type BoxGetSessionFuture = ExDataFuture<Option<BoxGetSessionFinish>>;

pub type BoxGetSessionFinish = Box<dyn FnOnce(&mut SslRef, &[u8]) -> Option<SslSession>>;

pub type BoxCustomVerifyFuture = ExDataFuture<Result<BoxCustomVerifyFinish, SslAlert>>;

pub type BoxCustomVerifyFinish = Box<dyn FnOnce(&mut SslRef) -> Result<(), SslAlert>>;

pub type ExDataFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub(crate) static TASK_WAKER_INDEX: LazyLock<Index<Ssl, Option<Waker>>> =
    LazyLock::new(|| Ssl::new_ex_index().unwrap());
pub(crate) static SELECT_CERT_FUTURE_INDEX: LazyLock<
    Index<Ssl, MutOnly<Option<BoxSelectCertFuture>>>,
> = LazyLock::new(|| Ssl::new_ex_index().unwrap());
pub(crate) static SELECT_PRIVATE_KEY_METHOD_FUTURE_INDEX: LazyLock<
    Index<Ssl, MutOnly<Option<BoxPrivateKeyMethodFuture>>>,
> = LazyLock::new(|| Ssl::new_ex_index().unwrap());
pub(crate) static SELECT_GET_SESSION_FUTURE_INDEX: LazyLock<
    Index<Ssl, MutOnly<Option<BoxGetSessionFuture>>>,
> = LazyLock::new(|| Ssl::new_ex_index().unwrap());
pub(crate) static SELECT_CUSTOM_VERIFY_FUTURE_INDEX: LazyLock<
    Index<Ssl, MutOnly<Option<BoxCustomVerifyFuture>>>,
> = LazyLock::new(|| Ssl::new_ex_index().unwrap());

impl SslContextBuilder {
    pub fn set_async_select_certificate_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ClientHello<'_>) -> Result<BoxSelectCertFuture, AsyncSelectCertError>
            + Send
            + Sync
            + 'static,
    {
        self.set_select_certificate_callback(move |mut client_hello| {
            let fut_poll_result = with_ex_data_future(
                &mut client_hello,
                *SELECT_CERT_FUTURE_INDEX,
                ClientHello::ssl_mut,
                &callback,
                identity,
            );

            let fut_result = match fut_poll_result {
                Poll::Ready(fut_result) => fut_result,
                Poll::Pending => return Err(SelectCertError::RETRY),
            };

            let finish = fut_result.or(Err(SelectCertError::ERROR))?;

            finish(client_hello).or(Err(SelectCertError::ERROR))
        });
    }

    pub fn set_async_private_key_method(&mut self, method: impl AsyncPrivateKeyMethod) {
        self.set_private_key_method(AsyncPrivateKeyMethodBridge(Box::new(method)));
    }

    pub unsafe fn set_async_get_session_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut SslRef, &[u8]) -> Option<BoxGetSessionFuture> + Send + Sync + 'static,
    {
        let async_callback = move |ssl: &mut SslRef, id: &[u8]| {
            let fut_poll_result = with_ex_data_future(
                &mut *ssl,
                *SELECT_GET_SESSION_FUTURE_INDEX,
                |ssl| ssl,
                |ssl| callback(ssl, id).ok_or(()),
                |option| option.ok_or(()),
            );

            match fut_poll_result {
                Poll::Ready(Err(())) => Ok(None),
                Poll::Ready(Ok(finish)) => Ok(finish(ssl, id)),
                Poll::Pending => Err(GetSessionPendingError),
            }
        };

        unsafe {
            self.set_get_session_callback(async_callback);
        }
    }

    pub fn set_async_custom_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(&mut SslRef) -> Result<BoxCustomVerifyFuture, SslAlert> + Send + Sync + 'static,
    {
        self.set_custom_verify_callback(mode, async_custom_verify_callback(callback));
    }
}

impl SslRef {
    pub fn set_async_custom_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(&mut SslRef) -> Result<BoxCustomVerifyFuture, SslAlert> + Send + Sync + 'static,
    {
        self.set_custom_verify_callback(mode, async_custom_verify_callback(callback));
    }

    pub fn set_task_waker(&mut self, waker: Option<Waker>) {
        self.replace_ex_data(*TASK_WAKER_INDEX, waker);
    }
}

fn async_custom_verify_callback<F>(
    callback: F,
) -> impl Fn(&mut SslRef) -> Result<(), SslVerifyError>
where
    F: Fn(&mut SslRef) -> Result<BoxCustomVerifyFuture, SslAlert> + Send + Sync + 'static,
{
    move |ssl| {
        let fut_poll_result = with_ex_data_future(
            &mut *ssl,
            *SELECT_CUSTOM_VERIFY_FUTURE_INDEX,
            |ssl| ssl,
            &callback,
            identity,
        );

        match fut_poll_result {
            Poll::Ready(Err(alert)) => Err(SslVerifyError::Invalid(alert)),
            Poll::Ready(Ok(finish)) => Ok(finish(ssl).map_err(SslVerifyError::Invalid)?),
            Poll::Pending => Err(SslVerifyError::Retry),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AsyncSelectCertError;

pub trait AsyncPrivateKeyMethod: Send + Sync + 'static {
    fn sign(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        signature_algorithm: SslSignatureAlgorithm,
        output: &mut [u8],
    ) -> Result<BoxPrivateKeyMethodFuture, AsyncPrivateKeyMethodError>;

    fn decrypt(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<BoxPrivateKeyMethodFuture, AsyncPrivateKeyMethodError>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AsyncPrivateKeyMethodError;

struct AsyncPrivateKeyMethodBridge(Box<dyn AsyncPrivateKeyMethod>);

impl PrivateKeyMethod for AsyncPrivateKeyMethodBridge {
    fn sign(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        signature_algorithm: SslSignatureAlgorithm,
        output: &mut [u8],
    ) -> Result<usize, PrivateKeyMethodError> {
        with_private_key_method(ssl, output, |ssl, output| {
            <dyn AsyncPrivateKeyMethod>::sign(&*self.0, ssl, input, signature_algorithm, output)
        })
    }

    fn decrypt(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<usize, PrivateKeyMethodError> {
        with_private_key_method(ssl, output, |ssl, output| {
            <dyn AsyncPrivateKeyMethod>::decrypt(&*self.0, ssl, input, output)
        })
    }

    fn complete(
        &self,
        ssl: &mut SslRef,
        output: &mut [u8],
    ) -> Result<usize, PrivateKeyMethodError> {
        with_private_key_method(ssl, output, |_, _| {
            if cfg!(debug_assertions) {
                panic!("BUG: boring called complete without a pending operation");
            }

            Err(AsyncPrivateKeyMethodError)
        })
    }
}

fn with_private_key_method(
    ssl: &mut SslRef,
    output: &mut [u8],
    create_fut: impl FnOnce(
        &mut SslRef,
        &mut [u8],
    ) -> Result<BoxPrivateKeyMethodFuture, AsyncPrivateKeyMethodError>,
) -> Result<usize, PrivateKeyMethodError> {
    let fut_poll_result = with_ex_data_future(
        ssl,
        *SELECT_PRIVATE_KEY_METHOD_FUTURE_INDEX,
        |ssl| ssl,
        |ssl| create_fut(ssl, output),
        identity,
    );

    let fut_result = match fut_poll_result {
        Poll::Ready(fut_result) => fut_result,
        Poll::Pending => return Err(PrivateKeyMethodError::RETRY),
    };

    let finish = fut_result.or(Err(PrivateKeyMethodError::FAILURE))?;

    finish(ssl, output).or(Err(PrivateKeyMethodError::FAILURE))
}

fn with_ex_data_future<H, R, T, E>(
    ssl_handle: &mut H,
    index: Index<Ssl, MutOnly<Option<ExDataFuture<R>>>>,
    get_ssl_mut: impl Fn(&mut H) -> &mut SslRef,
    create_fut: impl FnOnce(&mut H) -> Result<ExDataFuture<R>, E>,
    into_result: impl Fn(R) -> Result<T, E>,
) -> Poll<Result<T, E>> {
    let ssl = get_ssl_mut(ssl_handle);
    let waker = ssl
        .ex_data(*TASK_WAKER_INDEX)
        .cloned()
        .flatten()
        .expect("task waker should be set");

    let mut ctx = Context::from_waker(&waker);

    if let Some(data @ Some(_)) = ssl.ex_data_mut(index).map(MutOnly::get_mut) {
        let fut_result = into_result(ready!(data.as_mut().unwrap().as_mut().poll(&mut ctx)));

        *data = None;

        Poll::Ready(fut_result)
    } else {
        let mut fut = create_fut(ssl_handle)?;

        match fut.as_mut().poll(&mut ctx) {
            Poll::Ready(fut_result) => Poll::Ready(into_result(fut_result)),
            Poll::Pending => {
                get_ssl_mut(ssl_handle).replace_ex_data(index, MutOnly::new(Some(fut)));

                Poll::Pending
            }
        }
    }
}
