---
title: Your account
summary: Register, verify, reset a password, and sign out of a session.
audience: user
order: 20
---

## Register and verify

Create an account with your name, email, and a password of at least 8 characters. Until the address is verified you can sign in and look around, but you cannot push, vote, or manage packages.

Open the link in the mail, or paste the token with `taurctl verify TOKEN`. **Resend verification** on your account page sends another link.

On a brand-new server, with no site admin and with `TAUR_ADMINS` and `TAUR_OWNERS` unset, the first account becomes the site owner and is already verified.

## Password

**Forgot your password** on the sign-in page emails a one-hour link. Choosing a new password signs out every session. A site admin can send that link for you; they never see the token.

## Sessions

Your account page lists sessions. **Revoke** ends one of them. Ending the current session signs you out.

## Delete

Deleting the account asks for your password. Packages you maintain become orphans. You cannot delete the account while you are the only Owner of an organization.
