//! Writes of several requests: one transaction, in a Polarion session of its own.
//!
//! A library operation that needs more than one SOAP write (a create with follow-ups, an update
//! of standard and custom fields, ...) runs them through [`Polarion::atomic`]: in a session of its
//! own ([`Polarion::own_session`]), so the transaction never mixes with other calls sharing this
//! client's session, and in a spawned task, so it finishes (commit, or rollback on error) and
//! logs out even when the caller is dropped. When the runtime shuts down first, the server times
//! the session out with nothing committed.

use std::future::Future;

use crate::client::Polarion;
use crate::error::{Error, Result};

impl Polarion {
    /// Run `f`, which sends `writes` write requests for `operation`, as one transaction.
    ///
    /// - With at most one write, or inside this client's open transaction (an explicit one, or
    ///   the transaction of a session of its own calling further guarded methods), `f` runs on
    ///   this client.
    /// - Otherwise the transaction must be allowed (`check_transaction`); `f` gets a session of
    ///   its own, which writes only inside its transaction and logs in only once. The task:
    ///   `beginTransaction` (its login is the session's only one), `f`, then the commit, or a
    ///   rollback when `f` failed; the session is always ended. A commit refused by the server
    ///   committed nothing and returns its error; a commit without an answer is
    ///   [`Error::CommitUncertain`]. When the session expired in between, nothing is committed
    ///   and the result is [`polarion_soap::Error::SessionExpired`].
    ///
    /// The task is spawned on the current Tokio runtime (a panic in it is raised again here);
    /// without a runtime it runs in place.
    pub(crate) async fn atomic<T, F, Fut>(
        &self,
        operation: &'static str,
        writes: usize,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(Polarion) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        if writes <= 1 || self.soap().transaction_open() {
            return f(self.clone()).await;
        }
        self.check_transaction(operation)?;
        let session = self.own_session()?;
        let task = transaction(session, f);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => match runtime.spawn(task).await {
                Ok(result) => result,
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                // Only a runtime shutting down cancels the task.
                Err(_) => Err(Error::Invalid(format!(
                    "{operation}: the runtime shut down during the transaction; whether anything was committed is unknown"
                ))),
            },
            Err(_) => task.await,
        }
    }
}

/// The transaction of [`Polarion::atomic`] in the session of its own `session`.
async fn transaction<T, F, Fut>(session: Polarion, f: F) -> Result<T>
where
    F: FnOnce(Polarion) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let soap = session.soap().clone();
    let result = match soap.session().begin_transaction().await {
        Ok(()) => match f(session).await {
            Ok(value) if !soap.transaction_open() => {
                // The session expired, and its transaction with it; a commit would apply nothing.
                drop(value);
                Err(polarion_soap::Error::SessionExpired.into())
            }
            Ok(value) => match soap.session().end_transaction(false).await {
                Ok(()) => Ok(value),
                Err(
                    e @ (polarion_soap::Error::Http(_)
                    | polarion_soap::Error::Status { .. }
                    | polarion_soap::Error::Xml(_)
                    | polarion_soap::Error::Decode { .. }),
                ) => Err(Error::CommitUncertain(e)),
                // A fault (or an expired session): nothing was committed.
                Err(e) => Err(e.into()),
            },
            Err(e) => {
                if soap.transaction_open() {
                    // The error says more than a failed rollback; the session ends anyway.
                    let _ = soap.session().end_transaction(true).await;
                }
                Err(e)
            }
        },
        Err(e) => Err(e.into()),
    };
    // Ending the session only frees it early: an uncommitted transaction ends with it.
    let _ = soap.logout().await;
    result
}

// The integration tests' helpers. Declared here, not inside `tests`: a `#[path]` in an inline module
// is resolved from `src/atomic/tests/`, which does not exist, and Linux does not resolve `..`
// through a missing directory.
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod common;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{body_string_contains, path};
    use wiremock::{Mock, ResponseTemplate};

    use super::common::*;
    use super::*;

    const ITEM: &str = "subterra:data-service:objects:/default/Sandbox${WorkItem}SB-1";

    /// Two raw writes of a work item, as a write of several requests sends them.
    async fn two_writes(p: Polarion) -> Result<u32> {
        let tracker = p.soap().tracker();
        tracker.add_category(ITEM, "a").await?;
        tracker.add_category(ITEM, "b").await?;
        Ok(7)
    }

    async fn categories(server: &wiremock::MockServer, answer: ResponseTemplate) {
        Mock::given(path(TRACKER))
            .and(body_string_contains("addCategory>"))
            .respond_with(answer)
            .mount(server)
            .await;
    }

    fn added() -> ResponseTemplate {
        xml(
            "<addCategoryResponse><addCategoryReturn>true</addCategoryReturn></addCategoryResponse>",
        )
    }

    #[tokio::test]
    async fn writes_of_several_requests_run_in_a_transaction_of_their_own() {
        let server = session_server().await;
        categories(&server, added()).await;
        let polarion = writer(&server);
        polarion.login().await.unwrap();

        let result = polarion.atomic("op", 2, two_writes).await.unwrap();
        assert_eq!(result, 7);
        assert_eq!(
            log(&server).await,
            [
                ("", "logInWithToken"),
                ("", "logInWithToken"),
                ("-2", "beginTransaction"),
                ("-2", "addCategory"),
                ("-2", "addCategory"),
                ("-2", "endTransaction"),
                ("-2", "endSession"),
            ]
            .map(|(s, o)| (s.to_owned(), o.to_owned()))
        );
        let ends = bodies(&server, "endTransaction").await;
        assert!(ends[0].contains("rollback>false<"), "{}", ends[0]);
        // The main session is untouched.
        assert_eq!(polarion.soap().session_id().as_deref(), Some("-1"));
    }

    #[tokio::test]
    async fn single_writes_and_open_transactions_run_on_the_client() {
        let server = session_server().await;
        categories(&server, added()).await;
        let polarion = writer(&server);

        polarion
            .atomic("op", 1, |p| async move {
                p.soap().tracker().add_category(ITEM, "a").await?;
                Ok(())
            })
            .await
            .unwrap();
        polarion.begin_transaction().await.unwrap();
        polarion.atomic("op", 2, two_writes).await.unwrap();
        polarion.commit().await.unwrap();

        let log = log(&server).await;
        assert!(
            log.iter().all(|(s, _)| s == "-1" || s.is_empty()),
            "{log:?}"
        );
        assert_eq!(
            log.iter().filter(|(_, o)| o == "logInWithToken").count(),
            1,
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn failures_roll_back_and_end_the_session() {
        let server = session_server().await;
        Mock::given(path(TRACKER))
            .and(body_string_contains("addCategory>"))
            .and(body_string_contains("categoryId>b<"))
            .respond_with(fault("no such category"))
            .with_priority(1)
            .mount(&server)
            .await;
        categories(&server, added()).await;
        let polarion = writer(&server);

        let result = polarion.atomic("op", 2, two_writes).await;
        assert!(
            matches!(result, Err(Error::Soap(polarion_soap::Error::Fault { .. }))),
            "{result:?}"
        );
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(
            ops,
            [
                "logInWithToken",
                "beginTransaction",
                "addCategory",
                "addCategory",
                // The fault is checked against the session (`hasSubject`), which is valid.
                "hasSubject",
                "endTransaction",
                "endSession",
            ]
        );
        let ends = bodies(&server, "endTransaction").await;
        assert!(ends[0].contains("rollback>true<"), "{}", ends[0]);

        // An error of `f` without a request rolls back as well.
        let server = session_server().await;
        let result: Result<()> = writer(&server)
            .atomic("op", 2, |_| async { Err(Error::Invalid("no".into())) })
            .await;
        assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(
            ops,
            [
                "logInWithToken",
                "beginTransaction",
                "endTransaction",
                "endSession"
            ]
        );
    }

    #[tokio::test]
    async fn commits_refused_or_unanswered() {
        // Refused: nothing was committed, the fault is returned.
        let server = session_server().await;
        categories(&server, added()).await;
        Mock::given(path(SESSION))
            .and(body_string_contains("endTransaction>"))
            .respond_with(fault("conflict"))
            .with_priority(1)
            .mount(&server)
            .await;
        let result = writer(&server).atomic("op", 2, two_writes).await;
        assert!(
            matches!(result, Err(Error::Soap(polarion_soap::Error::Fault { .. }))),
            "{result:?}"
        );
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(
            ops.last().map(String::as_str),
            Some("endSession"),
            "{ops:?}"
        );
        assert_eq!(ops.iter().filter(|o| *o == "endTransaction").count(), 1);

        // No answer: the outcome is unknown.
        let server = session_server().await;
        categories(&server, added()).await;
        Mock::given(path(SESSION))
            .and(body_string_contains("endTransaction>"))
            .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
            .with_priority(1)
            .mount(&server)
            .await;
        let result = writer(&server).atomic("op", 2, two_writes).await;
        assert!(
            matches!(
                result,
                Err(Error::CommitUncertain(polarion_soap::Error::Status { .. }))
            ),
            "{result:?}"
        );
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(
            ops.last().map(String::as_str),
            Some("endSession"),
            "{ops:?}"
        );
        assert_eq!(ops.iter().filter(|o| *o == "endTransaction").count(), 1);
    }

    #[tokio::test]
    async fn failed_begins_send_nothing_else() {
        // The login fails: no session, nothing to end.
        let server = wiremock::MockServer::start().await;
        Mock::given(path(SESSION))
            .respond_with(fault("bad token"))
            .mount(&server)
            .await;
        let result = writer(&server).atomic("op", 2, two_writes).await;
        assert!(result.is_err());
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(ops, ["logInWithToken"]);

        // `beginTransaction` fails: the session is ended.
        let server = session_server().await;
        Mock::given(path(SESSION))
            .and(body_string_contains("beginTransaction>"))
            .respond_with(fault("no"))
            .with_priority(1)
            .mount(&server)
            .await;
        let result = writer(&server).atomic("op", 2, two_writes).await;
        assert!(result.is_err());
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert_eq!(
            ops,
            [
                "logInWithToken",
                "beginTransaction",
                "hasSubject",
                "endSession"
            ]
        );
    }

    #[tokio::test]
    async fn expired_sessions_commit_nothing() {
        let server = session_server().await;
        categories(&server, fault("Not authorized.")).await;
        Mock::given(path(SESSION))
            .and(body_string_contains("hasSubject>"))
            .respond_with(subject(false))
            .with_priority(1)
            .mount(&server)
            .await;
        let result = writer(&server).atomic("op", 2, two_writes).await;
        assert!(
            matches!(
                result,
                Err(Error::Soap(polarion_soap::Error::SessionExpired))
            ),
            "{result:?}"
        );

        // `f` swallowing the expiry still commits nothing.
        let server = session_server().await;
        categories(&server, fault("Not authorized.")).await;
        Mock::given(path(SESSION))
            .and(body_string_contains("hasSubject>"))
            .respond_with(subject(false))
            .with_priority(1)
            .mount(&server)
            .await;
        let result = writer(&server)
            .atomic("op", 2, |p| async move {
                let _ = two_writes(p).await;
                Ok(())
            })
            .await;
        assert!(
            matches!(
                result,
                Err(Error::Soap(polarion_soap::Error::SessionExpired))
            ),
            "{result:?}"
        );
        let ops: Vec<_> = log(&server).await.into_iter().map(|(_, o)| o).collect();
        assert!(!ops.iter().any(|o| o == "endTransaction"), "{ops:?}");
        assert_eq!(ops.iter().filter(|o| *o == "logInWithToken").count(), 1);
    }

    #[tokio::test]
    async fn dropped_callers_still_roll_back_and_end_the_session() {
        let server = session_server().await;
        Mock::given(path(TRACKER))
            .and(body_string_contains("addCategory>"))
            .respond_with(fault("no").set_delay(Duration::from_millis(300)))
            .mount(&server)
            .await;
        let polarion = writer(&server);
        let call = polarion.atomic("op", 2, two_writes);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), call)
                .await
                .is_err()
        );
        let mut ops = Vec::new();
        for _ in 0..100 {
            ops = log(&server).await.into_iter().map(|(_, o)| o).collect();
            if ops.last().is_some_and(|o| o == "endSession") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            ops,
            [
                "logInWithToken",
                "beginTransaction",
                "addCategory",
                "hasSubject",
                "endTransaction",
                "endSession"
            ]
        );
    }

    #[tokio::test]
    async fn concurrent_calls_have_sessions_of_their_own() {
        let server = session_server().await;
        categories(&server, added()).await;
        let polarion = writer(&server);
        polarion.login().await.unwrap();
        let (a, b) = tokio::join!(
            polarion.atomic("op", 2, two_writes),
            polarion.atomic("op", 2, two_writes)
        );
        a.unwrap();
        b.unwrap();
        let log = log(&server).await;
        for session in ["-2", "-3"] {
            let ops: Vec<_> = log
                .iter()
                .filter(|(s, _)| s == session)
                .map(|(_, o)| o.as_str())
                .collect();
            assert_eq!(
                ops,
                [
                    "beginTransaction",
                    "addCategory",
                    "addCategory",
                    "endTransaction",
                    "endSession"
                ],
                "{log:?}"
            );
        }
    }

    #[tokio::test]
    async fn refusals_send_nothing() {
        let server = session_server().await;
        // Read-only: no transaction.
        let result = reader(&server).atomic("op", 2, two_writes).await;
        assert!(
            matches!(result, Err(Error::WriteNotAllowed { .. })),
            "{result:?}"
        );
        // A wrapped SOAP client has no sessions of its own.
        let soap = polarion_soap::Client::builder(
            &base_url(&server),
            polarion_soap::Auth::Token("t".into()),
        )
        .build()
        .unwrap();
        let wrapped = Polarion::from_client(soap);
        let result = wrapped.atomic("op", 2, two_writes).await;
        assert!(result.is_err(), "{result:?}");
        assert!(log(&server).await.is_empty());
    }

    #[tokio::test]
    #[should_panic(expected = "boom")]
    async fn panics_are_raised_again() {
        let server = session_server().await;
        async fn boom(_: Polarion) -> Result<()> {
            panic!("boom")
        }
        let _ = writer(&server).atomic("op", 2, boom).await;
    }
}
