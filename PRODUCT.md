# PMEMC Product Context

## Current product boundary

PMEMC is a local Windows command-line application at a capability-free reset
point. The only enabled capability is the local `pmemc auth` credential
lifecycle.

PMEMC currently does not:

- open a conversational session;
- expose model tools;
- inspect, register, index, read, or write a repository;
- maintain project, memory, model, or other application data in a database;
- invoke CodeGraph, select source, or send content to an external service; or
- manage a model, provider, baseline, proposal, review, portfolio, or resume.

## Trust and safety boundary

- The operator controls credential setup and removal through `pmemc auth`.
- Credential input is secret and must be handled only by the local Windows
  credential store.
- PMEMC must never print, log, place in a repository, or persist that secret
  in an application database.
- Bare `pmemc` must not inspect the current directory or make an external
  request.

## Product promise

The reset establishes a clean, credential-safe baseline. No future agent or
repository workflow is implied by the current executable. Each future
capability must first receive an explicit contract, safety boundary, and test
suite.
