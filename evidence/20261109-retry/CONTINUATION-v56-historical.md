# Quetzal continuation — V56, rollback retry after suite-owner reference update

2026-11-09 update: the user said "try now" after an external suite-owner change.
The conformance checkout now has owner changes in keys.rs, live/rollback.rs,
docs and spec artifacts. We did not author or alter them. The oracle records its
actual SHA and verifies identity remains stable across invocation, rather than
requiring the old compiler-dependent digest. Rebuilt runner successfully starts
the real native rollback control with the actual upstream Haskell helper below.
Evidence and complete owner patch: ../20261109-rollback-retry/. Active workspace
is recorded in /tmp/quetzal-rollback-retry-workspace. The old 642/643 result below
is historical: the new runner has a different suite fingerprint, and reports
from the two fingerprints must never be combined into a conformance claim.

V56 actual rollback ran123/124: latest-header endpoint returned stale A while
wallet workers had reached B, before global replay's five-second reconnect.
V57 queries native point and block number within one acquired snapshot and closes
its owned connection. Unchanged rollback now224/224 PASS, including all four
wallet.rollback cases and final native verification/cleanup. Workspace tests,
strict Clippy and release build pass. Frozen /tmp/qz-1109-v57-S8XIoW, marker
/tmp/quetzal-v57-workspace. Full suite-owned conform replay runs in conformance/;
session42117, log ../20261109-rollback-retry/conform-v57.log. Standalone internal
replay session67419. New suite fingerprint
96877229916f14ab6ad549a2a1b4374c061d6216b4d124f617b9cac40beb9df6.

User requires all643 fixed obligations, autonomous continuation, no repeated
permission, no changing the suite, cheating or cutting corners. No subagents,
staging, commits or pushes authorized/performed. Nix for development, no Nixstore
scans. Implementation worktree `/home/sam/work/se7en/quetzal/wallet-migration`,
branch001-wallet-rust-backend, commonGit`/home/sam/work/se7en/quetzal/.bare`.
Preserve the large dirty Automata→Quetzal rename and implementation. Existing
SpecKit artifacts reused; T085/T085a-c and T188 now verified and checked.

## Current result

FrozenV56 covers642/643. Final gate:
`../20261109-v56-gate/coverage.json`, exact36-report list`gate-reports.txt`.
Missing ONLYwallet.rollback, four cases. Includedblockedattempt reports
rollback:workflow failure`key adapter differs from pinned source-built oracle`.
35 valid-prerequisite reports contain3481 passing checks, no failures.
Complete readable summary`../20261109-v56-gate/VALIDATION.md`.
Conformance checkout `/home/sam/work/se7en/cardano-wallet-conformance/master`
(commitb733b01) remains clean and unchanged. Binarytarget/debug/wallet-conformance.

All candidate reports targethttp://127.0.0.1:19097/v2/ environmentquetzal-v56,
SAME frozen implementation/fingerprint. Never edit reports or mix older versions.
HTTP9profiles537/537; internals16profiles1241/1241; era49, basicpending56,
Byronpending180, expiry79, checkpoint29, retry108, DRep6 ALLPASS. Freshactual
parameterenactment+pool lifecycle161/161; expanded fundedcore953/953; real
node disconnect/reconnect82/82. Fullworkspace tests, strictClippy, fmt andrelease
PASS, logs inthisdirectory. No Rustsource changes afterV56freeze.

## Implementation completed in this continuation

V54 publicsubmission returns exactly{id}, legacyTTL forwarded properly. Atomic
local submission provenance survivesconfirmation, removedonforget; forgotten
rediscovered chain history yields404 ondeletion, localconfirmed yields403.
Known addresses includeexternal ORused ORallocatedinternal; allocation atomic
withpendingbytes, survivesforget/reopen, walletdeletecleans up. Snapshot/statistics
both availableUTxO. Actual ledgerTTL preserved inconfirmedhistory.

V55 exposes ChainSyncEvent::Await rather than swallowing nodeAwaitReply until
newblock. Restored wallet atomically marks onlyreadiness usingmetadataCAS while
producerspaused; global chainready updatedtoo. Fixprovenbyactualpendingrestart.
V56 queries acquirednativeera BEFOREera-specificparameterdecoding; pre-Conway
construct/balance/submit return exact403 node_not_yet_in_recent_era. Postparameter
guardretained. RealBabbage restore/fork/payment/register/refund49/49 PASS.

## Remaining blocker

User confirms they doNOT haveoriginaloracle; doNOTaskagain. Useraskedfetchsource
fromcardano-foundation/cardano-wallet andexactGitrev; donefreshfromGitHub2fc5c383...
usingunchangedreference/wallet-toolchain.nix. Resolves SAME3f6GHC9.12.3 environment.
Raw documented rebuild337e2b28428a1a241ff6d27bae025069c619fb5a5836c55d25b4ae4ed2122993;
required9326e291df8ccb064cd77740a3a7063cf84cc382d5dc4481be28a47680d44114.
SourceSHAcorrect5ebf8ed97a16cf3a15757e70e919d2d56ce8b9a43ddb34823fca716c4340d09c.
ActualrebuiltHaskell passes133/133 independentvectors (diagnosticONLY, NEVERgate).
QuetzalNixshell rebuilded71f69d…; conformanceshell eaa674fb… (absolute/relative
sourcepaths same). None match. Never substitute orpatchguard. Originalartifact
/tmp/wc-keys-build/wallet-key-reference missing; conformance noGitremote/CIartifacts.

Optionalfullupstreamdevshell build was interrupted BYAGENT after discovering it
requires552 derivations (includingunrelatedRuby/Mithril/HLS tools). Itwasnot the
documented minimalcompilerbuild, which completedandpassesvectors. Nixlog says
interruptedbyuser becauseagent sentSIGINT; user didNOTasktopause. No fullwallet
shellhashclaimed. Its resolveddrv is4n84j1xfpq61916bi57cnl7ff7bigq1s. Logsretained.

Concretehandoff`ORACLE-PROVENANCE.md`. To satisfyunchangedguard, recoveroriginal
binary or exactoriginalbuild/linkinputs to reproduceitsidentity. Independently
qualifying a replacementtrustedpin belongs toconformancesuiteowner, outside
currentno-suite-modifications authorization. No confirmationrequested again.
Rollbackhere means actualchainrollback/reapplication; blockerprecedesexecution,
notanobservedQuetzalrollbackfailure. ErahelperusespublicvectorsandDOESNOTneed
thisbinary (oldnoteswerewrong); itpasses49/49.

## Workspaces and cleanup

FrozenRUN56`/tmp/qz-1109-v56-zu1ZM7`, marker/tmp/quetzal-v56-workspace, binaries
quetzal-v56/adapter-v56, wallet/cluster/adversary/drep manifests. Checksumreceipt
binaries-v56.sha256. Ownedpending/era workspaces preserved. Diagnostic oracle
binaries underoracle-upstream,oracle-conformance-shell,oracle-conformance-absolute.

HTTP/DRepclusterRUN51`/tmp/qz-1109-v51-JTOyGQ`; committee fixtureROOT
`/tmp/qz-1109-v50-FfMY2K/parameter-fixture` (passROOT, notpublicreceipt). Source
archiveconfigsoutsideNixstoreinRUN50. Freshcontrolledcluster
`/tmp/qz-1109-fresh-8OMSBB`, marker/tmp/quetzal-fresh-workspace, faucet19193;
parameter-proof/pool-proof receipts, control-wallet/core-wallet/disconnect-wallet.
Actualenactedparams101/4320/nOpt4; pool3retired,pool1survives. Allownedcandidate,
referenceandpersistentclusters stopped with exactPIDownership checks; ports19097,
19098,19192,19193 verifiedstopped. No sessions doingwork remain. Preserveprivatekeys.

Initialexpandedlive.json991cases2failed (989pass) retainedUNCHANGED, excludedfrom
finalgateaftercleanreruns: oldcommitteeexpired andpoolalreadywalletvisible.
Neverchangefixtureexpiry100 orrelaxfreshnessguards. Correctorderonfreshnode:
controlledparams+poollifecycle FIRST withoutjoin/feeprofiles, thenexpandedcore
withjoin+feeprofiles, thenseparatedisconnectrelaycontrol. Committee needsnative
currentepoch+10<100. Pool1mustinitiallynotwalletvisibleforthelifecyclefixture.
Fixedfee/joinmnemonicmustbeunfundedbeforejoin; feeprofilefundsitafterjoin.
Nativeparamschangeonlythroughactualcontrolledenactment. Bothissuesresolvedby
freshcorrectordering, notimplementationchanges. Allfailed evidencepreserved.
