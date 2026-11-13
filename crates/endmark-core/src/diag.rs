//! Findings: what a reader reports about a file.

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A reader will ignore something the author wrote, or a plain renderer
    /// will show something it should hide.
    Error,
    /// The file is valid but an item needs attention.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// The stable code of a finding. Codes do not change within a major version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    EarlyClose,
    UnterminatedRecord,
    UnterminatedFence,
    TrailingText,
    BadHeader,
    MissingAttribute,
    IncompleteEdit,
    MissingFence,
    UnexpectedFence,
    DuplicateId,
    UnknownTarget,
    WrongTarget,
    NotAuthor,
    AlreadyFinal,
    VisibleRecord,
    Detached,
    Ambiguous,
    Stale,
    DanglingReply,
    BodyAfterLog,
    UnknownVerb,
    UnknownAttribute,
    BadTimestamp,
}

impl Code {
    /// Every code, errors first, in the order of the RFC's table.
    pub const ALL: [Code; 23] = [
        Code::EarlyClose,
        Code::UnterminatedRecord,
        Code::UnterminatedFence,
        Code::TrailingText,
        Code::BadHeader,
        Code::MissingAttribute,
        Code::IncompleteEdit,
        Code::MissingFence,
        Code::UnexpectedFence,
        Code::DuplicateId,
        Code::UnknownTarget,
        Code::WrongTarget,
        Code::NotAuthor,
        Code::AlreadyFinal,
        Code::VisibleRecord,
        Code::Detached,
        Code::Ambiguous,
        Code::Stale,
        Code::DanglingReply,
        Code::BodyAfterLog,
        Code::UnknownVerb,
        Code::UnknownAttribute,
        Code::BadTimestamp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Code::EarlyClose => "early-close",
            Code::UnterminatedRecord => "unterminated-record",
            Code::UnterminatedFence => "unterminated-fence",
            Code::TrailingText => "trailing-text",
            Code::BadHeader => "bad-header",
            Code::MissingAttribute => "missing-attribute",
            Code::IncompleteEdit => "incomplete-edit",
            Code::MissingFence => "missing-fence",
            Code::UnexpectedFence => "unexpected-fence",
            Code::DuplicateId => "duplicate-id",
            Code::UnknownTarget => "unknown-target",
            Code::WrongTarget => "wrong-target",
            Code::NotAuthor => "not-author",
            Code::AlreadyFinal => "already-final",
            Code::VisibleRecord => "visible-record",
            Code::Detached => "detached",
            Code::Ambiguous => "ambiguous",
            Code::Stale => "stale",
            Code::DanglingReply => "dangling-reply",
            Code::BodyAfterLog => "body-after-log",
            Code::UnknownVerb => "unknown-verb",
            Code::UnknownAttribute => "unknown-attribute",
            Code::BadTimestamp => "bad-timestamp",
        }
    }

    pub fn severity(self) -> Severity {
        match self {
            Code::Detached
            | Code::Ambiguous
            | Code::Stale
            | Code::DanglingReply
            | Code::BodyAfterLog
            | Code::UnknownVerb
            | Code::UnknownAttribute
            | Code::BadTimestamp => Severity::Warning,
            _ => Severity::Error,
        }
    }

    /// How to fix it, in one line.
    pub fn hint(self) -> &'static str {
        match self {
            Code::EarlyClose => "write \"--\\>\" and \"--!\\>\" inside records",
            Code::UnterminatedRecord => "end the record with a line that is exactly \"-->\"",
            Code::UnterminatedFence => {
                "close every fence before the terminator, and write a \"-->\" line inside a fence as \"--\\>\""
            }
            Code::TrailingText => "put nothing after \"-->\" on a single-line record",
            Code::BadHeader => "the header is: <!--endmark VERB ID key=value ...",
            Code::MissingAttribute => "every record needs by= and at=",
            Code::IncompleteEdit => "follow each replace fence with a with fence",
            Code::MissingFence => "add the fence this verb requires, with content",
            Code::UnexpectedFence => "remove the fence, or use the verb it belongs to",
            Code::DuplicateId => "give the item a new id; the first use of an id wins",
            Code::UnknownTarget => "name an item that appears earlier in the file",
            Code::WrongTarget => "check which kind of item this verb acts on",
            Code::NotAuthor => "only the author of an item can retract it",
            Code::AlreadyFinal => "the item is closed; start a new one instead",
            Code::VisibleRecord => {
                "move the record to the end of the file, after any open code fence or HTML block is closed"
            }
            Code::Detached => "append an anchor record with a quote that is in the body, or resolve the thread",
            Code::Ambiguous => "append an anchor record with a longer quote, or add an in fence",
            Code::Stale => "retract the suggestion and suggest again against the current text",
            Code::DanglingReply => "set re= to the id of an item that appears earlier in the file",
            Code::BodyAfterLog => "move the text above the first record",
            Code::UnknownVerb => "readers ignore this record",
            Code::UnknownAttribute => "readers ignore this attribute",
            Code::BadTimestamp => "use RFC 3339 in UTC, such as 2026-11-13T09:30:00Z",
        }
    }
}

/// One finding about a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    /// 1-based line in the file.
    pub line: usize,
    /// The id of the item or record concerned, when it is known.
    pub id: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn new(code: Code, line: usize, id: Option<&str>, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            line,
            id: id.map(str::to_string),
            message: message.into(),
        }
    }

    pub fn severity(&self) -> Severity {
        self.code.severity()
    }

    pub fn hint(&self) -> &'static str {
        self.code.hint()
    }
}
