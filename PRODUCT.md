# PMEMC Product Context

## Current product boundary

PMEMC is a local Windows terminal application with two enabled capabilities:

- a local, full-screen UI preview opened by bare `pmemc`; and
- the `pmemc auth` credential lifecycle.

The UI preview accepts local text input and appends a fixed preview reply. It
does not read a credential or communicate with an agent or provider.

PMEMC currently does not:

- contact a model provider or stream model output;
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
- Bare `pmemc` must not inspect the current directory, read a credential, or
  make an external request.

## Product promise

The UI preview establishes a terminal interaction baseline without implying an
agent or repository workflow. Each future capability must receive an explicit
contract, safety boundary, and test suite.
