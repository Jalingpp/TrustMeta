// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Records compact commitments to TrustMeta EPRing root summaries.
/// @dev The full root summary remains off-chain. A verifier recomputes its
///      keccak256 digest and compares it with the event emitted by this contract.
contract EPRootProofRegistry {
    address public immutable manager;
    uint64 public epoch;

    mapping(bytes32 => bytes32) private latestSummaryDigests;

    event RootSummaryCommitted(
        uint64 indexed epoch,
        bytes32 indexed prefixDigest,
        bytes32 summaryDigest
    );

    event ManagerReset(uint64 indexed epoch);

    error Unauthorized(address caller);
    error LengthMismatch(uint256 prefixes, uint256 summaries);
    error EpochOverflow();

    constructor(address manager_) {
        require(manager_ != address(0), "manager is zero");
        manager = manager_;
    }

    modifier onlyManager() {
        if (msg.sender != manager) {
            revert Unauthorized(msg.sender);
        }
        _;
    }

    function commitBatch(
        bytes32[] calldata prefixDigests,
        bytes32[] calldata summaryDigests
    ) external onlyManager {
        if (prefixDigests.length != summaryDigests.length) {
            revert LengthMismatch(prefixDigests.length, summaryDigests.length);
        }

        for (uint256 i = 0; i < prefixDigests.length; ++i) {
            if (latestSummaryDigests[prefixDigests[i]] == summaryDigests[i]) {
                continue;
            }

            latestSummaryDigests[prefixDigests[i]] = summaryDigests[i];
            emit RootSummaryCommitted(epoch, prefixDigests[i], summaryDigests[i]);
        }
    }

    /// @dev The manager currently resets by recreating the entire dev chain.
    ///      This function preserves a lightweight logical reset API for callers
    ///      that choose to retain the chain in the future.
    function reset() external onlyManager {
        if (epoch == type(uint64).max) {
            revert EpochOverflow();
        }
        unchecked {
            ++epoch;
        }
        emit ManagerReset(epoch);
    }

    function latestCommitment(
        bytes32 prefixDigest
    ) external view returns (bytes32) {
        return latestSummaryDigests[prefixDigest];
    }
}
