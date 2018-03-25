//! A module providing a data structure for messages to and from IRC servers.
use std::borrow::ToOwned;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;

use error;
use error::{ProtocolError, MessageParseError};
use chan::ChannelExt;
use command::Command;

/// A data structure representing an IRC message according to the protocol specification. It
/// consists of a collection of IRCv3 tags, a prefix (describing the source of the message), and
/// the protocol command. If the command is unknown, it is treated as a special raw command that
/// consists of a collection of arguments and the special suffix argument. Otherwise, the command
/// is parsed into a more useful form as described in [Command](../command/enum.Command.html).
#[derive(Clone, PartialEq, Debug)]
pub struct OwnedMessage {
    /// Message tags as defined by [IRCv3.2](http://ircv3.net/specs/core/message-tags-3.2.html).
    /// These tags are used to add extended information to the given message, and are commonly used
    /// in IRCv3 extensions to the IRC protocol.
    pub tags: Option<Vec<Tag>>,
    /// The message prefix (or source) as defined by [RFC 2812](http://tools.ietf.org/html/rfc2812).
    pub prefix: Option<String>,
    /// The IRC command, parsed according to the known specifications. The command itself and its
    /// arguments (including the special suffix argument) are captured in this component.
    pub command: Command,
}

impl OwnedMessage {
    /// Creates a new message from the given components.
    ///
    /// # Example
    /// ```
    /// # extern crate irc_proto;
    /// # use irc_proto::OwnedMessage;
    /// # fn main() {
    /// let message = OwnedMessage::new(
    ///     Some("nickname!username@hostname"), "JOIN", vec!["#channel"], None
    /// ).unwrap();
    /// # }
    /// ```
    pub fn new(
        prefix: Option<&str>,
        command: &str,
        args: Vec<&str>,
        suffix: Option<&str>,
    ) -> Result<OwnedMessage, MessageParseError> {
        OwnedMessage::with_tags(None, prefix, command, args, suffix)
    }

    /// Creates a new IRCv3.2 message from the given components, including message tags. These tags
    /// are used to add extended information to the given message, and are commonly used in IRCv3
    /// extensions to the IRC protocol.
    pub fn with_tags(
        tags: Option<Vec<Tag>>,
        prefix: Option<&str>,
        command: &str,
        args: Vec<&str>,
        suffix: Option<&str>,
    ) -> Result<OwnedMessage, error::MessageParseError> {
        Ok(OwnedMessage {
            tags: tags,
            prefix: prefix.map(|s| s.to_owned()),
            command: Command::new(command, args, suffix)?,
        })
    }

    /// Gets the nickname of the message source, if it exists.
    ///
    /// # Example
    /// ```
    /// # extern crate irc_proto;
    /// # use irc_proto::OwnedMessage;
    /// # fn main() {
    /// let message = OwnedMessage::new(
    ///     Some("nickname!username@hostname"), "JOIN", vec!["#channel"], None
    /// ).unwrap();
    /// assert_eq!(message.source_nickname(), Some("nickname"));
    /// # }
    /// ```
    pub fn source_nickname(&self) -> Option<&str> {
        // <prefix> ::= <servername> | <nick> [ '!' <user> ] [ '@' <host> ]
        // <servername> ::= <host>
        self.prefix.as_ref().and_then(|s| match (
            s.find('!'),
            s.find('@'),
            s.find('.'),
        ) {
            (Some(i), _, _) | // <nick> '!' <user> [ '@' <host> ]
            (None, Some(i), _) => Some(&s[..i]), // <nick> '@' <host>
            (None, None, None) => Some(s), // <nick>
            _ => None, // <servername>
        })
    }

    /// Gets the likely intended place to respond to this message.
    /// If the type of the message is a `PRIVMSG` or `NOTICE` and the message is sent to a channel,
    /// the result will be that channel. In all other cases, this will call `source_nickname`.
    ///
    /// # Example
    /// ```
    /// # extern crate irc_proto;
    /// # use irc_proto::OwnedMessage;
    /// # fn main() {
    /// let msg1 = OwnedMessage::new(
    ///     Some("ada"), "PRIVMSG", vec!["#channel"], Some("Hi, everyone!")
    /// ).unwrap();
    /// assert_eq!(msg1.response_target(), Some("#channel"));
    /// let msg2 = OwnedMessage::new(
    ///     Some("ada"), "PRIVMSG", vec!["betsy"], Some("betsy: hi")
    /// ).unwrap();
    /// assert_eq!(msg2.response_target(), Some("ada"));
    /// # }
    /// ```
    pub fn response_target(&self) -> Option<&str> {
        match self.command {
            Command::PRIVMSG(ref target, _) if target.is_channel_name() => Some(target),
            Command::NOTICE(ref target, _) if target.is_channel_name() => Some(target),
            _ => self.source_nickname()
        }
    }

    /// Converts a OwnedMessage into a String according to the IRC protocol.
    ///
    /// # Example
    /// ```
    /// # extern crate irc_proto;
    /// # use irc_proto::OwnedMessage;
    /// # fn main() {
    /// let msg = OwnedMessage::new(
    ///     Some("ada"), "PRIVMSG", vec!["#channel"], Some("Hi, everyone!")
    /// ).unwrap();
    /// assert_eq!(msg.to_string(), ":ada PRIVMSG #channel :Hi, everyone!\r\n");
    /// # }
    /// ```
    pub fn to_string(&self) -> String {
        let mut ret = String::new();
        if let Some(ref tags) = self.tags {
            ret.push('@');
            for tag in tags {
                ret.push_str(&tag.0);
                if let Some(ref value) = tag.1 {
                    ret.push('=');
                    ret.push_str(value);
                }
                ret.push(';');
            }
            ret.pop();
            ret.push(' ');
        }
        if let Some(ref prefix) = self.prefix {
            ret.push(':');
            ret.push_str(prefix);
            ret.push(' ');
        }
        let cmd: String = From::from(&self.command);
        ret.push_str(&cmd);
        ret.push_str("\r\n");
        ret
    }
}

impl From<Command> for OwnedMessage {
    fn from(cmd: Command) -> OwnedMessage {
        OwnedMessage {
            tags: None,
            prefix: None,
            command: cmd,
        }
    }
}

impl FromStr for OwnedMessage {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<OwnedMessage, Self::Err> {
        if s.is_empty() {
            return Err(ProtocolError::InvalidMessage {
                string: s.to_owned(),
                cause: MessageParseError::EmptyMessage,
            })
        }

        let mut state = s;

        let tags = if state.starts_with('@') {
            let tags = state.find(' ').map(|i| &state[1..i]);
            state = state.find(' ').map_or("", |i| &state[i + 1..]);
            tags.map(|ts| {
                ts.split(';')
                    .filter(|s| !s.is_empty())
                    .map(|s: &str| {
                        let mut iter = s.splitn(2, '=');
                        let (fst, snd) = (iter.next(), iter.next());
                        Tag(fst.unwrap_or("").to_owned(), snd.map(|s| s.to_owned()))
                    })
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };

        let prefix = if state.starts_with(':') {
            let prefix = state.find(' ').map(|i| &state[1..i]);
            state = state.find(' ').map_or("", |i| &state[i + 1..]);
            prefix
        } else {
            None
        };

        let line_ending_len = if state.ends_with("\r\n") {
            "\r\n"
        } else if state.ends_with('\r') {
            "\r"
        } else if state.ends_with('\n') {
            "\n"
        } else {
            ""
        }.len();

        let suffix = if state.contains(" :") {
            let suffix = state.find(" :").map(|i| &state[i + 2..state.len() - line_ending_len]);
            state = state.find(" :").map_or("", |i| &state[..i + 1]);
            suffix
        } else {
            state = &state[..state.len() - line_ending_len];
            None
        };

        let command = match state.find(' ').map(|i| &state[..i]) {
            Some(cmd) => {
                state = state.find(' ').map_or("", |i| &state[i + 1..]);
                cmd
            }
            // If there's no arguments but the "command" starts with colon, it's not a command.
            None if state.starts_with(':') => return Err(ProtocolError::InvalidMessage {
                string: s.to_owned(),
                cause: MessageParseError::InvalidCommand,
            }),
            // If there's no arguments following the command, the rest of the state is the command.
            None => {
                let cmd = state;
                state = "";
                cmd
            },
        };

        let args: Vec<_> = state.splitn(14, ' ').filter(|s| !s.is_empty()).collect();

        OwnedMessage::with_tags(tags, prefix, command, args, suffix).map_err(|e| {
            ProtocolError::InvalidMessage {
                string: s.to_owned(),
                cause: e,
            }
        })
    }
}

impl<'a> From<&'a str> for OwnedMessage {
    fn from(s: &'a str) -> OwnedMessage {
        s.parse().unwrap()
    }
}

impl Display for OwnedMessage {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "{}", self.to_string())
    }
}

/// A message tag as defined by [IRCv3.2](http://ircv3.net/specs/core/message-tags-3.2.html).
/// It consists of a tag key, and an optional value for the tag. Each message can contain a number
/// of tags (in the string format, they are separated by semicolons). Tags are used to add extended
/// information to a message under IRCv3.
#[derive(Clone, PartialEq, Debug)]
pub struct Tag(pub String, pub Option<String>);

#[cfg(test)]
mod test {
    use super::{OwnedMessage, Tag};
    use command::Command::{PRIVMSG, QUIT, Raw};

    #[test]
    fn new() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: PRIVMSG(format!("test"), format!("Testing!")),
        };
        assert_eq!(
            OwnedMessage::new(None, "PRIVMSG", vec!["test"], Some("Testing!")).unwrap(),
            message
        )
    }

    #[test]
    fn source_nickname() {
        assert_eq!(
            OwnedMessage::new(None, "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            None
        );

        assert_eq!(
            OwnedMessage::new(Some("irc.test.net"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            None
        );

        assert_eq!(
            OwnedMessage::new(Some("test!test@test"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );

        assert_eq!(
            OwnedMessage::new(Some("test@test"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );

        assert_eq!(
            OwnedMessage::new(Some("test!test@irc.test.com"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );

        assert_eq!(
            OwnedMessage::new(Some("test!test@127.0.0.1"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );

        assert_eq!(
            OwnedMessage::new(Some("test@test.com"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );

        assert_eq!(
            OwnedMessage::new(Some("test"), "PING", vec![], Some("data"))
                .unwrap()
                .source_nickname(),
            Some("test")
        );
    }

    #[test]
    fn to_string() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: PRIVMSG(format!("test"), format!("Testing!")),
        };
        assert_eq!(&message.to_string()[..], "PRIVMSG test :Testing!\r\n");
        let message = OwnedMessage {
            tags: None,
            prefix: Some(format!("test!test@test")),
            command: PRIVMSG(format!("test"), format!("Still testing!")),
        };
        assert_eq!(
            &message.to_string()[..],
            ":test!test@test PRIVMSG test :Still testing!\r\n"
        );
    }

    #[test]
    fn from_string() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: PRIVMSG(format!("test"), format!("Testing!")),
        };
        assert_eq!(
            "PRIVMSG test :Testing!\r\n".parse::<OwnedMessage>().unwrap(),
            message
        );
        let message = OwnedMessage {
            tags: None,
            prefix: Some(format!("test!test@test")),
            command: PRIVMSG(format!("test"), format!("Still testing!")),
        };
        assert_eq!(
            ":test!test@test PRIVMSG test :Still testing!\r\n"
                .parse::<OwnedMessage>()
                .unwrap(),
            message
        );
        let message = OwnedMessage {
            tags: Some(vec![
                Tag(format!("aaa"), Some(format!("bbb"))),
                Tag(format!("ccc"), None),
                Tag(format!("example.com/ddd"), Some(format!("eee"))),
            ]),
            prefix: Some(format!("test!test@test")),
            command: PRIVMSG(format!("test"), format!("Testing with tags!")),
        };
        assert_eq!(
            "@aaa=bbb;ccc;example.com/ddd=eee :test!test@test PRIVMSG test :Testing with \
                    tags!\r\n"
                .parse::<OwnedMessage>()
                .unwrap(),
            message
        )
    }

    #[test]
    fn from_string_atypical_endings() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: PRIVMSG(format!("test"), format!("Testing!")),
        };
        assert_eq!(
            "PRIVMSG test :Testing!\r".parse::<OwnedMessage>().unwrap(),
            message
        );
        assert_eq!(
            "PRIVMSG test :Testing!\n".parse::<OwnedMessage>().unwrap(),
            message
        );
        assert_eq!(
            "PRIVMSG test :Testing!".parse::<OwnedMessage>().unwrap(),
            message
        );
    }

    #[test]
    fn from_and_to_string() {
        let message = "@aaa=bbb;ccc;example.com/ddd=eee :test!test@test PRIVMSG test :Testing with \
                       tags!\r\n";
        assert_eq!(message.parse::<OwnedMessage>().unwrap().to_string(), message);
    }

    #[test]
    fn to_message() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: PRIVMSG(format!("test"), format!("Testing!")),
        };
        let msg: OwnedMessage = "PRIVMSG test :Testing!\r\n".into();
        assert_eq!(msg, message);
        let message = OwnedMessage {
            tags: None,
            prefix: Some(format!("test!test@test")),
            command: PRIVMSG(format!("test"), format!("Still testing!")),
        };
        let msg: OwnedMessage = ":test!test@test PRIVMSG test :Still testing!\r\n".into();
        assert_eq!(msg, message);
    }

    #[test]
    fn to_message_with_colon_in_arg() {
        // Apparently, UnrealIRCd (and perhaps some others) send some messages that include
        // colons within individual parameters. So, let's make sure it parses correctly.
        let message = OwnedMessage {
            tags: None,
            prefix: Some(format!("test!test@test")),
            command: Raw(
                format!("COMMAND"),
                vec![format!("ARG:test")],
                Some(format!("Testing!")),
            ),
        };
        let msg: OwnedMessage = ":test!test@test COMMAND ARG:test :Testing!\r\n".into();
        assert_eq!(msg, message);
    }

    #[test]
    fn to_message_no_prefix_no_args() {
        let message = OwnedMessage {
            tags: None,
            prefix: None,
            command: QUIT(None),
        };
        let msg: OwnedMessage = "QUIT\r\n".into();
        assert_eq!(msg, message);
    }

    #[test]
    #[should_panic]
    fn to_message_invalid_format() {
        let _: OwnedMessage = ":invalid :message".into();
    }
}
