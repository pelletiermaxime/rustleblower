# Guided system-firewall statements and verdicts

The optional guided editor complements the complete raw statement/value editor.
It stages only a local rule. Keeping the rule still goes through the existing
complete-document review, captured node/session/fingerprint guard, asynchronous
RELOAD_FW_RULES command and confirmed cache update. It never executes a local
firewall command or selects a different chain implicitly.

Source contract: retained OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`,
`ui/opensnitch/dialogs/firewall_rule/statements.py:17-292,367-433`,
`dialog.py:353-557`, `rules.py:96-118,124-185`, and
`ui/opensnitch/firewall/{enums,exprs}.py`. New implementation is
`src/firewall/helpers.rs` (pure bounded construction) and
`src/app/firewall_view/helpers.rs` (pinned GPUI Kit Select/Input/Button controls),
integrated into `forms.rs`. No second GUI backend is introduced.

## User flow

On a rule's Expressions tab, choose **Add guided statement…**, select a family,
its protocol/family/metadata/logging/over-or-until option, comparison and value.
The contextual hint explains the selected format. **Keep in rule draft** adds the
validated expression at the end of the ordered expressions and selects its page.
**Cancel helper** or Escape preserves the rule. Ctrl+Enter keeps the active helper,
consistent with the raw nested editor. Helpers never replace an existing unknown
statement; raw Edit remains available for all supported wire fields.

On the Rule tab, **Choose guided verdict…** constructs a supported target and
parameters for the captured chain hook. Keep updates the visible raw fields;
Cancel preserves them. Queue uses `num N`; supported NAT targets use `to IP:port`
or `to :port`. SNAT requires postrouting; redirect/TPROXY/DNAT are refused on
input/postrouting. DNAT/redirect additionally require output/prerouting: the source
UI callback implicitly changes direction to make those combinations reachable;
the native helper refuses instead of changing a captured chain. The helper never
moves rules to a new hook/table. Existing raw fields can represent other daemon values.

## Implemented families and exact encoding

| Family | Ordered wire construction |
| --- | --- |
| Source/destination ports | A single protocol uses its statement name and sport/dport value. `tcp,udp` uses `meta` with l4proto followed by sport/dport. Numeric single ports, inclusive ranges and comma lists are validated. |
| Source/destination address | `ip`/`ip6`, saddr/daddr. Canonical CIDR is converted arithmetically to first–last, including `/0` and IPv6; no host enumeration/allocation. Family mismatch, noncanonical host bits and reversed ranges are refused. |
| Input/output interface | iifname/oifname, interface text in the **Key** and empty Value, as required by the pinned constructor. |
| Set/match connection mark | `ct`, optional empty `set` marker then `mark`. |
| Set packet mark | `meta`, empty `set` then `mark`. |
| Connection states | `ct`, `state` with new/established/related/invalid comma list. |
| Packet information | `meta`, chosen mark/l4proto/skuid/skgid/protocol/priority key. Zero is permitted, unlike the other guided numeric fields. |
| ICMP / ICMPv6 | icmp/icmpv6, `type`, all source-listed names including IPv6 neighbour discovery. |
| Log | `log`, level then prefix. Source ASCII-space removal is retained and disclosed in the UI. |
| Quota | `quota`, optional empty `over` then the selected byte unit mapped to its count. Until omits the flag. |
| Named counter | `counter`, empty packets marker then name. |
| Rate limit | `limit`, optional empty `over`, then units/rate-units/time-units. Until omits the flag. |

Six source comparisons are available for matching statements; action statements
use their normal source default `==`. Helper text is at most 1024 bytes, one line,
with no NUL. There are at most 128 port terms; marks/packet numeric values fit
u32, counts fit u64, and port/queue values use the corresponding finite domain.
The existing complete-firewall payload/allocation limits are checked before an
expression is retained. Input controls share the existing 8-KiB raw-field bound;
accepted helper values have the tighter 1-KiB bound. Seventeen finite family
option sets and two additional selectors are retained per guided statement;
closing it drops its entities. There is no new I/O worker or event queue.

## Proof and limits

`tests/firewall_helpers.rs` has source-derived expected wire fixtures for all 17
families, invalid/cap/address-domain cases, arithmetic `/0` conversion, verdict
hook/parameter checks and a complete protobuf/document roundtrip that preserves
an existing unknown raw expression, legacy fields, UUID and uint64 position.
Run `python scripts/cargo.py test --no-default-features --locked --test firewall_helpers`.
These are constructor/interchange checks; actual rendered control input and
daemon/kernel enforcement are separate evidence.

Dedicated GPU selectors: `fw-expression-guided`, `fw-helper-kind`,
`fw-helper-option`, `fw-helper-operator`, `fw-helper-value`, `fw-helper-keep`,
`fw-helper-cancel`, `fw-verdict-guided`, `fw-helper-verdict` and
`fw-helper-verdict-value`. Verify invalid-value preservation, cancellation,
dual-protocol and IPv6 encoding, verdict hook refusal, normal complete review,
held ACK with independent permission input and exact confirmed cache contents.
The shared verification owner ran the expanded 18-frame firewall-forms journey:
`.verification/headless/20261017T022741Z-b0a175e2/result.json` passed. Actual
input covers dual-protocol `!=` with `443,8443`, IPv6 CIDR conversion, explicit
SNAT/postrouting refusal on an output chain, queue `num 12`, complete payload,
held ACK/independent permission and reconnect assertions. Frames 15–18 were
visually inspected. The first attempt is preserved at
`.verification/headless/20261017T022336Z-66b482c8/`: it exposed Kit Select's
`size_full` host consuming surplus flex height. Explicit bounded trigger rows
fixed the hit bounds; no constructor/protocol assertion was weakened.

The original four core cases also pass in the complete 200-case owned-namespace run;
the final focused run passes five helper cases including guided DNAT/redirect
hook refusal without moving the rule. Its transport/helper aggregate and source
hashes are in `.verification/remote-transport-20261016/final-focused-result.json`.
The retained `full-core.log` and immutable core hashes in `full-core-result.json`
are under `.verification/remote-transport-20261016/`. Background
GPU/input/protocol/cache proof is distinct from native compositor helper input
or kernel enforcement.

Local service/interface suggestions now have production code, two fixture core
cases and a desktop compilation pass. Discovery stays off the UI thread, reads
at most 256 KiB/2,048 protocol-qualified service records and 128 local interface
entries, and refuses symlink/special service files. `/usr/etc/services` is the
fallback when `/etc/services` is absent; the pinned WireGuard UDP preset is
retained. Searchable Kit suggestions fill numeric port plus protocol, or a local
interface name. Typed service aliases require an unambiguous port for every
selected protocol; numeric single/list/range and manual interface/wildcard values
remain available. Reload preserves the entered value and updates only suggestion
pickers. These interfaces belong to this computer, including when editing a
remote node; no remote interface inventory is guessed. Input/wire proof for these
controls now passes in the 22-frame `firewall-forms` GPU journey at
`.verification/headless/20261017T030013Z-f8b4702d/result.json`. New frames 19–22
exercise actual ambiguous-alias refusal, exact UDP port/protocol and local
interface picks, and owned file reload preserving the typed value before the
new suggestion is used. The complete expected wire snapshot, held ACK, separate
permission reply and stale reconnect remain asserted. All PNGs and owned
process/socket cleanup pass. Aggregate/scoped hashes:
`.verification/firewall-suggestions-result.json`. A stale port hint in these
screenshots is a subsequent copy-only correction.

The source's
commented-out masquerade preset is not offered. Advanced raw verdict parameters
(including reject variants), unusual signed values, unknown statements and
source data outside the helper's stricter numeric/address constraints remain
editable through the existing raw fields. These restrictions apply only to the
new guided constructors; imports and raw edits are not silently normalized.
Native compositor proof, backend/kernel enforcement and the final whole-app
sorting/color/layout pass remain separate.
