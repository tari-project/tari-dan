# Wallet sequence diagram

```mermaid
sequenceDiagram
participant User
participant TariConnector
participant SignalingServer
participant WalletUI
participant WalletDaemon

User->>TariConnector: User clicks "Connect" button
TariConnector->>SignalingServer: auth.login(permissions)
SignalingServer->>SignalingServer: generate JWT with increasing ID + permissions
SignalingServer->>TariConnector: returns signaling server JWT
TariConnector->>TariConnector: Create and store webRTC offer in memory (hashmap)
TariConnector->>User: Show QR that contains the JWT 
User->>User: Copy JWT to clipboard

User->>WalletUI: User clicks "Connect" button with the JWT in the clipboard
WalletUI->>User: Displays modal to review the requested permissions
User->>WalletUI: User accepts the permissions
WalletUI->>WalletDaemon: webrtc.start(JWT)
WalletDaemon->>WalletDaemon: Check that the caller has the StartWebrtc permission
WalletDaemon->>WalletDaemon: Parse the JWT, extract permissions and generate a permission token
WalletDaemon->>WalletDaemon: Spawn tokio task to handle the WebRTC channel, using the permission token

WalletDaemon->>SignalingServer: WebRTC communications to set up the channel

User->>TariConnector: User clicks "SetAnswer" button
TariConnector->>SignalingServer: getAnswer
TariConnector->>SignalingServer: getIceCandidates
TariConnector->>TariConnector: create the data channel with the Ice candidates
TariConnector->>SignalingServer: WebRTC communications to set up the channel

User->>TariConnector: sendMessage(walletDaemonMethod, JWT, args)
TariConnector->>TariConnector: generate a new messageId = previousMessageId + 1
TariConnector->>SignalingServer: WebRTC messaging with the user request
SignalingServer->>WalletDaemon: WebRTC messaging with the user request
WalletDaemon->>WalletDaemon: execute the request and return the result
WalletDaemon->>SignalingServer: WebRTC messaging with the response
SignalingServer->>TariConnector: WebRTC messaging with the response
TariConnector->>User: response