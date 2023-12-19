# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

## [0.3.0](https://github.com/tari-project/tari-dan/compare/v0.2.0...v0.3.0) (2023-12-19)


### ⚠ BREAKING CHANGES

* libp2p (#827)

### Features

* add version to template WASMs ([#835](https://github.com/tari-project/tari-dan/issues/835)) ([8612eab](https://github.com/tari-project/tari-dan/commit/8612eab9a1e6a713b04f86e624c5501fcf1c1808))
* do fee estimation in UI transfer ([#826](https://github.com/tari-project/tari-dan/issues/826)) ([93bfd45](https://github.com/tari-project/tari-dan/commit/93bfd452bd33fe8138d98df164bddbe7642ed650))
* libp2p ([#827](https://github.com/tari-project/tari-dan/issues/827)) ([9c29995](https://github.com/tari-project/tari-dan/commit/9c29995cf0e3f5e7bbb875ea20e02dfa20eab540))
* **p2p:** peer-sync protocol ([#844](https://github.com/tari-project/tari-dan/issues/844)) ([b49af42](https://github.com/tari-project/tari-dan/commit/b49af421ec3cb72af6df42a952e26eeb4c286c03))
* request foreign blocks ([#760](https://github.com/tari-project/tari-dan/issues/760)) ([7a59c4d](https://github.com/tari-project/tari-dan/commit/7a59c4d4d2f3d3dcf55880e9a3fd12a5a73dc25e))
* show dummy blocks in ui ([#843](https://github.com/tari-project/tari-dan/issues/843)) ([d5c77f6](https://github.com/tari-project/tari-dan/commit/d5c77f6e2dbcaa9518343bc453df77c56924e219))


### Bug Fixes

* claim burn in the ui ([#841](https://github.com/tari-project/tari-dan/issues/841)) ([ca80982](https://github.com/tari-project/tari-dan/commit/ca80982672e4849f52ee5befca8e5e2e7106a003))
* cli argument duplicate ([#837](https://github.com/tari-project/tari-dan/issues/837)) ([cb2d694](https://github.com/tari-project/tari-dan/commit/cb2d694feb259683a0c58697b6d37d55c6a91867))
* force txs refetch on account change in UI ([#833](https://github.com/tari-project/tari-dan/issues/833)) ([3e09ad5](https://github.com/tari-project/tari-dan/commit/3e09ad5a2bb00dc4e309a9874f968cd17c34f7ed))
* **p2p/messaging:** single stream per connection ([#845](https://github.com/tari-project/tari-dan/issues/845)) ([c0e09fe](https://github.com/tari-project/tari-dan/commit/c0e09fefffaee7666c55c36025c039026109f21d))
* **swarm:** exit with error if unsupported seed multiaddr ([#836](https://github.com/tari-project/tari-dan/issues/836)) ([b54bde8](https://github.com/tari-project/tari-dan/commit/b54bde8178883a49038aa9b0ce6f57450e7184d6))

## [0.2.0](https://github.com/tari-project/tari-dan/compare/v0.1.1...v0.2.0) (2023-12-08)


### ⚠ BREAKING CHANGES

* foreign broadcast reliability counter (#757)

### Features

* add transaction json download to ui ([#815](https://github.com/tari-project/tari-dan/issues/815)) ([50c0ff5](https://github.com/tari-project/tari-dan/commit/50c0ff5e5bacbcc2deb221b0cd55f42f61174551))
* disable buttons on send, add result dialog ([#813](https://github.com/tari-project/tari-dan/issues/813)) ([1d146b8](https://github.com/tari-project/tari-dan/commit/1d146b8190696b58dab6dbdae6abe8132319ea97))
* foreign broadcast reliability counter ([#757](https://github.com/tari-project/tari-dan/issues/757)) ([f0dc999](https://github.com/tari-project/tari-dan/commit/f0dc99954f634a8ac995a65bf06837edacede808))
* foreign proposal command ([#792](https://github.com/tari-project/tari-dan/issues/792)) ([186b20d](https://github.com/tari-project/tari-dan/commit/186b20d338cd3ee2c152037a6f4ba806148e44eb))
* **integration_tests:** new test for downed substates ([#798](https://github.com/tari-project/tari-dan/issues/798)) ([5a0c47a](https://github.com/tari-project/tari-dan/commit/5a0c47af80c5690869be218afdb1415742be4317))
* proper transaction signature and validation ([#791](https://github.com/tari-project/tari-dan/issues/791)) ([e6a1082](https://github.com/tari-project/tari-dan/commit/e6a108215c6e88a1e79738914aa89489836faf9f))
* set refresh balance interval to 5 sec ([#819](https://github.com/tari-project/tari-dan/issues/819)) ([61dfa4d](https://github.com/tari-project/tari-dan/commit/61dfa4d996854910712b050970fdbc5c18496942))
* show substate version in dan wallet ui ([#810](https://github.com/tari-project/tari-dan/issues/810)) ([89b2879](https://github.com/tari-project/tari-dan/commit/89b287987109b26da70eed596185145d9f4afe24))
* sort TXs in UI, add timestamp ([#804](https://github.com/tari-project/tari-dan/issues/804)) ([7dad32e](https://github.com/tari-project/tari-dan/commit/7dad32ec1e8cac548b88d1d0bd4e4fe41d0db89a))


### Bug Fixes

* indexer settings in dan wallet ui ([#805](https://github.com/tari-project/tari-dan/issues/805)) ([068d1ad](https://github.com/tari-project/tari-dan/commit/068d1ad1a3cd4b9eb1a378694dc9714febca1b85))
* propagation ([#799](https://github.com/tari-project/tari-dan/issues/799)) ([ef10627](https://github.com/tari-project/tari-dan/commit/ef10627ea77af78d9c4799dd115b164f2507e942))
* shard range computation ([#796](https://github.com/tari-project/tari-dan/issues/796)) ([892fe0c](https://github.com/tari-project/tari-dan/commit/892fe0ce871e6c1a8a9f70d9c51ec196f86cd175))
* shorten string on small strings ([#823](https://github.com/tari-project/tari-dan/issues/823)) ([064c540](https://github.com/tari-project/tari-dan/commit/064c54067ce09b798022bda2e0bdcbbe7a31bb8e))
* **wallet_daemon_web_ui:** send correct max_fee param on transfers ([#795](https://github.com/tari-project/tari-dan/issues/795)) ([0f07b81](https://github.com/tari-project/tari-dan/commit/0f07b8161ce6493d76d549fc2fd1b8dd9d38dfd2))
