use mio::tcp::{TcpStream};
use mio::{Token, TryRead, TryWrite, PollOpt, EventSet, EventLoop, Handler};
use mio::util::Slab;

use bytes::{Buf, Take};

use std::mem;
use std::io::Cursor;
use std::net::SocketAddr;
use std::io::Read;
use std::fs::OpenOptions;

const PING: Token = Token(0);
const MAX_LINE: usize = 128;

struct Ping {
    connections: TcpStream,
}

impl Ping {
    pub fn new(connections: TcpStream) -> Ping {
        Ping {
            connections: connections,
        }
    }

    pub fn start(address: SocketAddr) {
        let connections = TcpStream::connect(&address).expect("Unable to bind socket");
        let mut event_loop = EventLoop::new().expect("Unable to create event loop");
        event_loop.register(&connections, PING,
                            EventSet::readable(),
                            PollOpt::edge()).expect("Unable to register event loop");
        let mut ping = Ping::new(connections);
        event_loop.run(&mut ping).expect("Unable to run event loop");
    }
}

impl Handler for Ping {
    type Timeout = ();
    type Message = ();

    fn ready(&mut self, 
        event_loop: &mut EventLoop<Ping>,
        token: Token,
        events: EventSet) {
            match token {
                PING => {
                    println!("The Client is ready to accept a connection");
                    match events.is_readable() {
                        true => {
                            let mut vec: Vec<u8> = Vec::new();
                            self.connections.read_to_end(&mut vec);
                            event_loop.shutdown();
                        },
                        false => {},
                    }
                }

                _ => {
                    event_loop.shutdown();
                }
            }
        
    }
}

#[derive(Debug)]
struct Connection {
    socket: TcpStream,
    token: Token,
    state: State,
}

impl Connection {
    fn new(socket: TcpStream, token: Token) -> Connection {
        Connection {
            socket: socket,
            token: token,
            state: State::Reading(Vec::with_capacity(MAX_LINE)),
        }
    }

    fn ready(&mut self, event_loop: &mut EventLoop<Ping>, events: EventSet) {
        match self.state {
            State::Reading(..) => {
                assert!(events.is_readable(), "unexpected events; events={:?}", events);
                self.read(event_loop)
            }
            State::Writing(..) => {
                assert!(events.is_writable(), "unexpected events; events={:?}", events);
                self.write(event_loop)
            }
            _ => unimplemented!(),
        }
    }

    fn read(&mut self, event_loop: &mut EventLoop<Ping>) {
        match self.socket.try_read_buf(self.state.mut_read_buf()) {
            Ok(Some(0)) => {
                self.state = State::Closed;
            }
            Ok(Some(n)) => {
                println!("read {} bytes", n);

                // Look for a new line. If a new line is received, then the
                // state is transitioned from `Reading` to `Writing`.
                self.state.try_transition_to_writing();
                // Re-register the socket with the event loop. The current
                // state is used to determine whether we are currently reading
                // or writing.
                self.reregister(event_loop);
            }
            Ok(None) => {
                self.reregister(event_loop);
            }
            Err(e) => {
                panic!("got an error trying to read; err={:?}", e);
            }
        }
    }

    fn write(&mut self, event_loop: &mut EventLoop<Ping>) {
        // TODO: handle error
        match self.socket.try_write_buf(self.state.mut_write_buf()) {
            Ok(Some(_)) => {
                // If the entire line has been written, transition back to the
                // reading state
                self.state.try_transition_to_reading();

                // Re-register the socket with the event loop.
                self.reregister(event_loop);
            }
            Ok(None) => {
                // The socket wasn't actually ready, re-register the socket
                // with the event loop
                self.reregister(event_loop);
            }
            Err(e) => {
                panic!("got an error trying to write; err={:?}", e);
            }
        }
    }

    fn reregister(&self, event_loop: &mut EventLoop<Ping>) {
        event_loop.reregister(&self.socket, self.token, self.state.event_set(), PollOpt::oneshot())
            .unwrap();
    }

    fn is_closed(&self) -> bool {
        match self.state {
            State::Closed => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
enum State {
    Reading(Vec<u8>),
    Writing(Take<Cursor<Vec<u8>>>),
    Closed,
}

impl State {
    fn mut_read_buf(&mut self) -> &mut Vec<u8> {
        match *self {
            State::Reading(ref mut buf) => buf,
            _ => panic!("connection not in reading state"),
        }
    }

    fn read_buf(&self) -> &[u8] {
        match *self {
            State::Reading(ref buf) => buf,
            _ => panic!("connection not in reading state"),
        }
    }

    fn write_buf(&self) -> &Take<Cursor<Vec<u8>>> {
        match *self {
            State::Writing(ref buf) => buf,
            _ => panic!("connection not in writing state"),
        }
    }

    fn mut_write_buf(&mut self) -> &mut Take<Cursor<Vec<u8>>> {
        match *self {
            State::Writing(ref mut buf) => buf,
            _ => panic!("connection not in writing state"),
        }
    }

    // Looks for a new line, if there is one the state is transitioned to
    // writing
    fn try_transition_to_writing(&mut self) {
        if let Some(pos) = self.read_buf().iter().position(|b| *b == b'\n') {
            // First, remove the current read buffer, replacing it with an
            // empty Vec<u8>.
            let buf = mem::replace(self, State::Closed)
                .unwrap_read_buf();

            // Wrap in `Cursor`, this allows Vec<u8> to act as a readable
            // buffer
            let buf = Cursor::new(buf);

            // Transition the state to `Writing`, limiting the buffer to the
            // new line (inclusive).
            *self = State::Writing(Take::new(buf, pos + 1));
        }
    }

    // If the buffer being written back to the client has been consumed, switch
    // back to the reading state. However, there already might be another line
    // in the read buffer, so `try_transition_to_writing` is called as a final
    // step.
    fn try_transition_to_reading(&mut self) {
        if !self.write_buf().has_remaining() {
            let cursor = mem::replace(self, State::Closed)
                .unwrap_write_buf()
                .into_inner();

            let pos = cursor.position();
            let mut buf = cursor.into_inner();

            // Drop all data that has been written to the client
            drain_to(&mut buf, pos as usize);

            *self = State::Reading(buf);

            // Check for any new lines that have already been read.
            self.try_transition_to_writing();
        }
    }

    fn event_set(&self) -> EventSet {
        match *self {
            State::Reading(..) => EventSet::readable(),
            State::Writing(..) => EventSet::writable(),
            _ => EventSet::none(),
        }
    }

    fn unwrap_read_buf(self) -> Vec<u8> {
        match self {
            State::Reading(buf) => buf,
            _ => panic!("connection not in reading state"),
        }
    }

    fn unwrap_write_buf(self) -> Take<Cursor<Vec<u8>>> {
        match self {
            State::Writing(buf) => buf,
            _ => panic!("connection not in writing state"),
        }
    }
}

fn drain_to(vec: &mut Vec<u8>, count: usize) {
    // A very inefficient implementation. A better implementation could be
    // built using `Vec::drain()`, but the API is currently unstable.
    for _ in 0..count {
        vec.remove(0);
    }
}
