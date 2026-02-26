// Ego-Kernel On-Chain Program: Reputation Ledger + Micro-Escrow
//
// Deployed on Solana via Anchor Framework 0.31.0.
// This is the "Court" — the immutable legal layer of the Sovereign Protocol.
//
// # Instructions
// - `initialize_kernel`: Mint a new KernelProfile (Soulbound identity).
// - `log_interaction`: Record a success or betrayal; update reputation score.
// - `flag_bad_faith`: Permanently attach a signed Dishonesty Hash.
// - `create_escrow`: Open a micro-escrow for a pending negotiation.
// - `settle_escrow`: Settle the escrow on success or refund on betrayal.

use anchor_lang::prelude::*;

declare_id!("EgoKernelReputation11111111111111111111111");

// ─── Constants ────────────────────────────────────────────────────────────────

pub const BASELINE_REPUTATION: u64 = 1_000;
pub const BETRAYAL_MULTIPLIER: u64 = 5;
pub const EXILE_THRESHOLD: u64 = 100;
pub const SOVEREIGN_THRESHOLD: u64 = 980;
pub const STABLE_THRESHOLD: u64 = 500;
pub const MAX_FLAGS: usize = 64;

// ─── Program ──────────────────────────────────────────────────────────────────

#[program]
pub mod ego_kernel_reputation {
    use super::*;

    /// Initialize a new Kernel identity with the baseline Reputation Score.
    /// This mints the "Genesis Block" for the user's Ego-Kernel.
    pub fn initialize_kernel(ctx: Context<Initialize>, uid: String) -> Result<()> {
        let profile = &mut ctx.accounts.kernel_profile;
        profile.authority = ctx.accounts.user.key();
        profile.uid = uid;
        profile.reputation_score = BASELINE_REPUTATION;
        profile.total_interactions = 0;
        profile.total_successes = 0;
        profile.total_betrayals = 0;
        profile.is_exiled = false;
        profile.bad_faith_flags = 0;
        profile.initialized_at = Clock::get()?.unix_timestamp;
        emit!(KernelInitialized {
            authority: ctx.accounts.user.key(),
            uid: profile.uid.clone(),
            timestamp: profile.initialized_at,
        });
        Ok(())
    }

    /// Log an interaction outcome — either a Success or a Betrayal.
    /// Betrayals cost 5× the impact of a success (permanent ledger hit).
    pub fn log_interaction(
        ctx: Context<UpdateReputation>,
        success: bool,
        impact: u64,
    ) -> Result<()> {
        let profile = &mut ctx.accounts.kernel_profile;
        require!(!profile.is_exiled, EKError::KernelExiled);

        if success {
            profile.reputation_score = profile.reputation_score.saturating_add(impact);
            profile.total_successes += 1;
        } else {
            let penalty = impact.saturating_mul(BETRAYAL_MULTIPLIER);
            profile.reputation_score = profile.reputation_score.saturating_sub(penalty);
            profile.total_betrayals += 1;
        }

        profile.total_interactions += 1;

        // Exile check: the ultimate deterrent.
        if profile.reputation_score < EXILE_THRESHOLD {
            profile.is_exiled = true;
            emit!(KernelExiledEvent {
                authority: profile.authority,
                uid: profile.uid.clone(),
                final_score: profile.reputation_score,
                timestamp: Clock::get()?.unix_timestamp,
            });
        }

        emit!(InteractionLogged {
            authority: profile.authority,
            success,
            impact,
            new_score: profile.reputation_score,
            is_exiled: profile.is_exiled,
        });

        Ok(())
    }

    /// Attach a signed Dishonesty Hash to a Kernel's ledger.
    /// This permanently raises the Trust Tax on all of the target's future transactions.
    pub fn flag_bad_faith(
        ctx: Context<FlagTarget>,
        dishonesty_hash: [u8; 32],
    ) -> Result<()> {
        let target = &mut ctx.accounts.target_profile;
        require!(!target.is_exiled, EKError::KernelExiled);
        require!(
            (target.bad_faith_flags as usize) < MAX_FLAGS,
            EKError::FlagLimitReached
        );

        target.bad_faith_flags += 1;
        // Each flag increases the cost of all future transactions by ~40%.
        let penalty = 50u64.saturating_mul(target.bad_faith_flags as u64);
        target.reputation_score = target.reputation_score.saturating_sub(penalty);

        if target.reputation_score < EXILE_THRESHOLD {
            target.is_exiled = true;
        }

        emit!(BadFaithFlagged {
            target: target.authority,
            flagger: ctx.accounts.flagger.key(),
            dishonesty_hash,
            new_score: target.reputation_score,
        });

        Ok(())
    }

    /// Open a micro-escrow for a pending negotiation.
    /// Funds are locked until `settle_escrow` is called by the authority.
    pub fn create_escrow(ctx: Context<CreateEscrow>, amount: u64, description: String) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        escrow.authority = ctx.accounts.user.key();
        escrow.counterparty = ctx.accounts.counterparty.key();
        escrow.amount = amount;
        escrow.description = description;
        escrow.is_settled = false;
        escrow.created_at = Clock::get()?.unix_timestamp;

        // Transfer lamports into the escrow PDA.
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.escrow.key(),
            amount,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.escrow.to_account_info(),
            ],
        )?;

        emit!(EscrowCreated {
            authority: escrow.authority,
            counterparty: escrow.counterparty,
            amount,
        });

        Ok(())
    }

    /// Settle an escrow: release funds to the counterparty on success,
    /// or refund the authority on betrayal.
    pub fn settle_escrow(ctx: Context<SettleEscrow>, success: bool) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(!escrow.is_settled, EKError::AlreadySettled);

        escrow.is_settled = true;

        let amount = escrow.amount;
        if success {
            // Release to counterparty.
            **escrow.to_account_info().try_borrow_mut_lamports()? -= amount;
            **ctx.accounts.counterparty.try_borrow_mut_lamports()? += amount;
        } else {
            // Refund to authority.
            **escrow.to_account_info().try_borrow_mut_lamports()? -= amount;
            **ctx.accounts.authority.try_borrow_mut_lamports()? += amount;
        }

        emit!(EscrowSettled {
            authority: escrow.authority,
            counterparty: escrow.counterparty,
            amount,
            success,
        });

        Ok(())
    }
}

// ─── Account Structs ──────────────────────────────────────────────────────────

#[account]
pub struct KernelProfile {
    pub authority: Pubkey,           // 32
    pub uid: String,                 // 4 + 64
    pub reputation_score: u64,       // 8
    pub total_interactions: u64,     // 8
    pub total_successes: u64,        // 8
    pub total_betrayals: u64,        // 8
    pub bad_faith_flags: u8,         // 1
    pub is_exiled: bool,             // 1
    pub initialized_at: i64,         // 8
}

impl KernelProfile {
    pub const LEN: usize = 8 + 32 + (4 + 64) + 8 + 8 + 8 + 8 + 1 + 1 + 8;

    pub fn tier(&self) -> &'static str {
        if self.reputation_score >= SOVEREIGN_THRESHOLD {
            "SOVEREIGN"
        } else if self.reputation_score >= STABLE_THRESHOLD {
            "STABLE"
        } else if self.reputation_score >= EXILE_THRESHOLD {
            "VOLATILE"
        } else {
            "EXILED"
        }
    }
}

#[account]
pub struct EscrowAccount {
    pub authority: Pubkey,    // 32
    pub counterparty: Pubkey, // 32
    pub amount: u64,          // 8
    pub description: String,  // 4 + 128
    pub is_settled: bool,     // 1
    pub created_at: i64,      // 8
}

impl EscrowAccount {
    pub const LEN: usize = 8 + 32 + 32 + 8 + (4 + 128) + 1 + 8;
}

// ─── Instruction Contexts ─────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = user,
        space = KernelProfile::LEN,
        seeds = [b"kernel", user.key().as_ref()],
        bump,
    )]
    pub kernel_profile: Account<'info, KernelProfile>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateReputation<'info> {
    #[account(mut, has_one = authority)]
    pub kernel_profile: Account<'info, KernelProfile>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct FlagTarget<'info> {
    #[account(mut)]
    pub target_profile: Account<'info, KernelProfile>,
    pub flagger: Signer<'info>,
    // Flagger must have a valid (non-exiled) kernel to flag others.
    #[account(has_one = authority @ EKError::Unauthorized)]
    pub flagger_profile: Account<'info, KernelProfile>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct CreateEscrow<'info> {
    #[account(
        init,
        payer = user,
        space = EscrowAccount::LEN,
        seeds = [b"escrow", user.key().as_ref(), counterparty.key().as_ref()],
        bump,
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: counterparty is a plain wallet receiving funds.
    pub counterparty: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleEscrow<'info> {
    #[account(mut, has_one = authority)]
    pub escrow: Account<'info, EscrowAccount>,
    pub authority: Signer<'info>,
    /// CHECK: counterparty receives released funds.
    #[account(mut)]
    pub counterparty: AccountInfo<'info>,
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[event]
pub struct KernelInitialized {
    pub authority: Pubkey,
    pub uid: String,
    pub timestamp: i64,
}

#[event]
pub struct InteractionLogged {
    pub authority: Pubkey,
    pub success: bool,
    pub impact: u64,
    pub new_score: u64,
    pub is_exiled: bool,
}

#[event]
pub struct KernelExiledEvent {
    pub authority: Pubkey,
    pub uid: String,
    pub final_score: u64,
    pub timestamp: i64,
}

#[event]
pub struct BadFaithFlagged {
    pub target: Pubkey,
    pub flagger: Pubkey,
    pub dishonesty_hash: [u8; 32],
    pub new_score: u64,
}

#[event]
pub struct EscrowCreated {
    pub authority: Pubkey,
    pub counterparty: Pubkey,
    pub amount: u64,
}

#[event]
pub struct EscrowSettled {
    pub authority: Pubkey,
    pub counterparty: Pubkey,
    pub amount: u64,
    pub success: bool,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[error_code]
pub enum EKError {
    #[msg("Kernel is in EXILE state. Automated blacklisting active.")]
    KernelExiled,
    #[msg("Unauthorized: caller does not control this kernel profile.")]
    Unauthorized,
    #[msg("Escrow has already been settled.")]
    AlreadySettled,
    #[msg("Flag limit reached for this target.")]
    FlagLimitReached,
}
