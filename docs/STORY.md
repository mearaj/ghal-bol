# Story, anything in the docs that violates this story should be overriden and this story should be preferred over it

After user login for the first time then background service (ghal_bol) should
start running. It should watch the network continuously, should know the status 
of the internet, should be quick to figure out it's global reachable address and also it's 
LAN address and as soon as it's global reachable address is found it should regularly
register itself at the coord server. WAN should always work if internet is active for
both the peers and if coord server is reachable. Now if any peer is found on LAN then only 
for that peer LAN should be used and in case if LAN is lost then again it should repeat the
retular process of WAN and this switch shouldn't impact user experience, he shouldn't see any
weird behavior. Now in case if coord server is unreachable then it shouldn't fall back to libp2p **for WAN peer
discovery** (no Kademlia, no public bootstrap peer directory) like before — coord/relay is
required for WAN lookup. **libp2p stays** for transport: relay circuits, NAT hole-punch
(DCUtR), mDNS on LAN, Noise streams, ping, AutoNAT. LAN ability must not be impacted when
coord HTTP is down, and the app must keep retrying all configured coord servers on a regular
interval. The ultimate goal is strong, reliable and smooth interaction between peers. We already
have the coord/relay server(s) and libp2p which should be more than enough for smooth interaction
over the WAN/LAN.
The current biggest issue is that agent fails to understand it and it always break 
atleast one functionality.It doesn't understands the importance of keeping the p2p active
in every possible scenario where there's possiblity for both to reach each other.
For example if a peer left the LAN then it takes too much time to find despite the fact 
that we have coord/relay server and also libp2p.
Another example is if both the peer are online and if they found they are on LAN then 
their connection should immmediately shift to LAN because it is much faaster and stronger.
All this what I am saying is a very common sense things which the agents lacks too much.
We don't want to use bootstrap peers and Kademlia to **find peers over WAN** via libp2p as we
did before — coord/relay is the WAN directory now. libp2p is still used wherever transport needs
it (relay reservation, hole-punch, mDNS, streams); we avoid libp2p's peer-directory behaviour
that floods the network.
Second change: instead of a single coord/relay server, the app accepts an **array** of coord
servers via `GHAL_BOL_COORD_URLS` in
`env/.env.development` and `env/.env.production` — no hardcoded URLs in code. For now each
list has one entry; more can be added later.
If multiple coord/relay server exists, then app is suppose to try register on all those
and if the app is looking for peers then it should try searching on those multiple servers
and on if it's successfull on any server on finding and connecting to the peer then no
need to try on subsequent servers, but if anytime the connection drop between the 
peers and internet is active then it should repeat this process.
