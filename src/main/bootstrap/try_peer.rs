// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use common::{Core, Message, Priority, Socket, State};
use main::PeerId;
use mio::{EventLoop, EventSet, PollOpt, Token};
use sodiumoxide::crypto::box_::PublicKey;
use std::net::SocketAddr;

// TODO(Spandan) Result contains socket address too as currently due to bug in mio we are unable to
// obtain peer address from a connected socket. Track https://github.com/carllerche/mio/issues/397
// and remove this once that is solved.
pub type Finish = Box<FnMut(&mut Core,
                            &mut EventLoop<Core>,
                            Token,
                            Result<(Socket, SocketAddr, PeerId), SocketAddr>)>;

pub struct TryPeer {
    token: Token,
    peer: SocketAddr,
    socket: Option<Socket>,
    request: Option<(Message, Priority)>,
    finish: Finish,
}

impl TryPeer {
    pub fn start(core: &mut Core,
                 el: &mut EventLoop<Core>,
                 peer: SocketAddr,
                 our_pk: PublicKey,
                 name_hash: u64,
                 finish: Finish)
                 -> ::Res<Token> {
        let socket = try!(Socket::connect(&peer));
        let token = core.get_new_token();

        let state = TryPeer {
            token: token,
            peer: peer,
            socket: Some(socket),
            request: Some((Message::BootstrapRequest(our_pk, name_hash), 0)),
            finish: finish,
        };

        try!(el.register(unwrap!(state.socket.as_ref()),
                         token,
                         EventSet::error() | EventSet::hup() | EventSet::writable(),
                         PollOpt::edge()));

        let _ = core.insert_state(token, Rc::new(RefCell::new(state)));

        Ok(token)
    }

    fn write(&mut self,
             core: &mut Core,
             el: &mut EventLoop<Core>,
             msg: Option<(Message, Priority)>) {
        if unwrap!(self.socket.as_mut()).write(el, self.token, msg).is_err() {
            self.handle_error(core, el);
        }
    }

    fn receive_response(&mut self, core: &mut Core, el: &mut EventLoop<Core>) {
        match unwrap!(self.socket.as_mut()).read::<Message>() {
            Ok(Some(Message::BootstrapResponse(peer_pk))) => {
                let _ = core.remove_state(self.token);
                let token = self.token;
                let data = (unwrap!(self.socket.take()), self.peer, PeerId(peer_pk));
                (*self.finish)(core, el, token, Ok(data));
            }
            Ok(None) => (),
            Ok(Some(_)) | Err(_) => self.handle_error(core, el),
        }
    }

    fn handle_error(&mut self, core: &mut Core, el: &mut EventLoop<Core>) {
        self.terminate(core, el);
        let token = self.token;
        let peer = self.peer;
        (*self.finish)(core, el, token, Err(peer));
    }
}

impl State for TryPeer {
    fn ready(&mut self, core: &mut Core, el: &mut EventLoop<Core>, es: EventSet) {
        if es.is_error() || es.is_hup() {
            self.handle_error(core, el);
        } else {
            if es.is_writable() {
                let req = self.request.take();
                self.write(core, el, req);
            }
            if es.is_readable() {
                self.receive_response(core, el)
            }
        }
    }

    fn terminate(&mut self, core: &mut Core, el: &mut EventLoop<Core>) {
        let _ = core.remove_state(self.token);
        let _ = el.deregister(&unwrap!(self.socket.take()));
    }

    fn as_any(&mut self) -> &mut Any {
        self
    }
}
