# Pls Run some jobs

# Problem

Running arbitrary linux processes on remote machine.

# Approach

## Library

Individual commands are started as child processes. Handle is owned by `Job`, which streams logs of the process to a file and collects exit code. As a consequence, in the absence of cleanup policy _chatty_ processes would quickly exhaust available disk space. 

Jobs are grouped together and owned by `Controller`, which serves as a container for jobs. Controller, deals with various aspects of spawning and responds to queries. 

## Cgroups

Library leverages cgroups to enforce resource limits. V2 is chosen for its simpler design (single hierarchy). Since controllers could be part of only one hierarchy and systemd mounts some controllers to v1 by default, `cgroup_no_v1=all` kernel boot param is used to opt-out of this behaviour and use unified (v2) hierarchy instead. See [playbook](./playbook.yml) for implementation details.

Following controls are supported:
- `cpu.weight`: provide value in [1, 100000] to set weight for spawned process (default 100), relative to other client processes.
- `memory.high`: provide value in bytes to cap memory, process using more than this value will be throttled.
- `memory.max`: provide value in bytes to hard-cap memory, process using more than this value will be OomKilled. 
- `io.max.rbps` & `io.max.wbps`: provide value to cap io in bytes per second. 

### Running as root

Adding process to a cgroup requires eid of the process performing the move to be the owner of common ancestor of source and destination cgroups. In other words, for child C of process P belonging to non-root user U, to be moved in a cgroup owned by U, P itself needs to be in a cgroup that is direct ancestor of target cgroup and owned by U.   
This could be achieved by setting up appropriate permissions beforehand as privileged user and executing binary as U.
For this implementation it is a no-goal and process is ran as root. 

## Authn/z

Authentication is implemented with mTLS. In a production scenario job-runner service provider would leverage their own CA, to generate chain of trust. Each client (in the business sense, as an organization) could be issued intermediate CA, which in return would be used to issue end entity certificates. 
Combination of `O`rganization & `emailAddress` values in the subject field could be used for fine-grained control of client (and end entity associated with such client) capabilities. 
For this implementation such flexibility is a no-goal, instead authentication is implemented with a simpler scheme of single root CA, issuing end-entity certificates for client and server. 

Authorization is done via job ownership scheme, leveraging `O`rganization value read from the subject field of the end-entity certificate provided by caller.

System uses TLS v1.2 cipher suite:
- 2048 RSA keys, since it is current industry standard (and some cloud providers do not support 4096 keys yet).
- sha256 for message authentication 

Application leverages [rustls](https://github.com/rustls/rustls) for crypto needs, which had been [audited by Cure53](https://github.com/rustls/rustls/blob/master/audit/TLS-01-report.pdf) on behalf of CNCF in 2020. 

## gRPC 

[tonic](https://github.com/hyperium/tonic) is default gRPC server/client library in Rust ecosystem.

<details>
<summary> Click to view protobuf </summary>

```protobuf
syntax = "proto3";

package runner;

service JobRunner {
  rpc Start(JobRequest) returns (JobId);
  rpc Stop(JobId) returns (Ack);
  rpc Status(JobId) returns (JobStatus);
  rpc Output(JobId) returns (stream LogMessage);
}

message JobRequest {
  message CpuControl { uint32 cpu_weight = 1; }

  message MemControl {
    uint64 mem_high = 1;
    uint64 mem_max = 2;
  }

  message IoControl {
    uint64 rbps_max = 1;
    uint64 wbps_max = 2;
  }

  string executable = 1;
  optional CpuControl cpu_control = 2;
  optional MemControl mem_control = 3;
  optional IoControl io_control = 4;
  repeated string args = 5;
}

message Ack {}

message JobId { bytes jobid = 1; }

message JobStatus {
  oneof outcome {
    int32 exit_code = 1;
    int32 signal = 2;
  }
}

message LogMessage {
  enum Fd {
    out = 0;
    err = 1;
  }
  Fd fd = 1;
  bytes output = 2;
}
```
</details>


## Pics 

### Server startup state transition 

[![](https://mermaid.ink/img/eyJjb2RlIjoiZ3JhcGggVERcbiAgICBTW1N0YXJ0dXBdLS0-UkNcbiAgICBSQ1tSZWFkaW5nIENlcnRzXVxuICAgIFJDLS0-fEdvdCB1c2VycyBmcm9tIGNlcnRzIE9yZ2FuaXphdGlvbiB2YWx1ZXxFVVxuICAgIFJDLS0-fGNlcnQgcmVhZCBmYWlsfEVbRmF0YWwgZXJyb3IgLT4gcHJvY2Vzcy5leGl0XVxuICAgIEVVW0Vuc3VyaW5nIE9yZy1sZXZlbCBVc2Vyc11cbiAgICBFVS0tPnxVc2VycyBleGlzdHxDQ0dbQ3JlYXRpbmcgdXNlciBjZ3JvdXBzXVxuICAgIEVVLS0-fFVzZXJzIGRvbid0IGV4aXN0fFVBKHVzZXJhZGQpXG4gICAgVUEtLT58Q3JlYXRlZHxDQ0dcbiAgICBVQS0tPnx1c2VyYWRkIGZhaWx8RSBcbiAgICBDQ0ctLT58Y2dyb3VwcyBva3xDRFtDcmVhdGluZyB1c2VyIGRpcnNdXG4gICAgQ0NHLS0-fGNncm91cHMgZmFpbHxFIFxuICAgIENELS0-fERpcnMgb2t8TFtMaXN0ZW4gZm9yIGluY29taW5nIHJlcXVlc3RzXVxuICAgIENELS0-fERpcnMgZmFpbHxFXG4iLCJtZXJtYWlkIjp7InRoZW1lIjoiZGVmYXVsdCJ9LCJ1cGRhdGVFZGl0b3IiOmZhbHNlLCJhdXRvU3luYyI6dHJ1ZSwidXBkYXRlRGlhZ3JhbSI6ZmFsc2V9)](https://mermaid.live/edit#eyJjb2RlIjoiZ3JhcGggVERcbiAgICBTW1N0YXJ0dXBdLS0-UkNcbiAgICBSQ1tSZWFkaW5nIENlcnRzXVxuICAgIFJDLS0-fEdvdCB1c2VycyBmcm9tIGNlcnRzIE9yZ2FuaXphdGlvbiB2YWx1ZXxFVVxuICAgIFJDLS0-fGNlcnQgcmVhZCBmYWlsfEVbRmF0YWwgZXJyb3IgLT4gcHJvY2Vzcy5leGl0XVxuICAgIEVVW0Vuc3VyaW5nIE9yZy1sZXZlbCBVc2Vyc11cbiAgICBFVS0tPnxVc2VycyBleGlzdHxDQ0dbQ3JlYXRpbmcgdXNlciBjZ3JvdXBzXVxuICAgIEVVLS0-fFVzZXJzIGRvbid0IGV4aXN0fFVBKHVzZXJhZGQpXG4gICAgVUEtLT58Q3JlYXRlZHxDQ0dcbiAgICBVQS0tPnx1c2VyYWRkIGZhaWx8RSBcbiAgICBDQ0ctLT58Y2dyb3VwcyBva3xDRFtDcmVhdGluZyB1c2VyIGRpcnNdXG4gICAgQ0NHLS0-fGNncm91cHMgZmFpbHxFIFxuICAgIENELS0-fERpcnMgb2t8TFtMaXN0ZW4gZm9yIGluY29taW5nIHJlcXVlc3RzXVxuICAgIENELS0-fERpcnMgZmFpbHxFXG4iLCJtZXJtYWlkIjoie1xuICBcInRoZW1lXCI6IFwiZGVmYXVsdFwiXG59IiwidXBkYXRlRWRpdG9yIjpmYWxzZSwiYXV0b1N5bmMiOnRydWUsInVwZGF0ZURpYWdyYW0iOmZhbHNlfQ)

Notes:
- Upon parsing certificates server process has knowledge of finite set of clients who will be calling it. This knoweldge is leveraged to create distinct linux users named after `Organization` value in subject field. 
- For each of those users, base directory (for jobs to run in) and organization-level cgroups are created. Information about created users, cgroups and dirs is stored on Controller associated with specific user.
- Upon receiving `Start(JobRequest)`, server process picks the organization name from the supplied certificate and dispatches request to controller for that specific user. 

<details>
<summary> Server startup state transition mermaid </summary>

```txt
graph TD
    S[Startup]-->RC
    RC[Reading Certs]
    RC-->|Got users from certs Organization value|EU
    RC-->|cert read fail|E[Fatal error -> process.exit]
    EU[Ensuring Org-level Users]
    EU-->|Users exist|CCG[Creating user cgroups]
    EU-->|Users don't exist|UA(useradd)
    UA-->|Created|CCG
    UA-->|useradd fail|E 
    CCG-->|cgroups ok|CD[Creating user dirs]
    CCG-->|cgroups fail|E 
    CD-->|Dirs ok|L[Listen for incoming requests]
    CD-->|Dirs fail|E
```
</details>

### Start Job


[![](https://mermaid.ink/img/eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBKIGFzIEpvYlxucGFydGljaXBhbnQgQ1JBIGFzIENvbnRyb2xsZXIgZm9yIENsaWVudCBBXG5wYXJ0aWNpcGFudCBGUyBhcyBGaWxlIFN0b3JhZ2VcblxuQ0EtPj4rUzogc3RhcnQ6IDxicj5jdXJsIGV4YW1wbGUuY29tIDxicj5jcHUud2VpZ2h0PTUwMFxuUy0-PkNSQTogbmV3IEpvYiByZXF1ZXN0IGZvciBDbGllbnQgQVxuQ1JBLT4-SjogbmV3KClcbkotPj5DUkE6IEpvYklkXG5DUkEtPj5DUkE6IHNldCB1cCBjZ3JvdXAgY29udHJvbHMgZm9yIEpvYklkXG5DUkEtPj5KOiBmb3JrKClcbkotPj5KOiBXcml0ZSBnZXRwaWQoKSB0byBjZ3JvdXAucHJvY3NcbkotPj5KOiBzZXRnaWQoKSBhbmQgc2V0dWlkKCkgdG8gQ2xpZW50IEEgdXNlclxuSi0-Pko6IGV4ZWMoKVxuSi0-PkNSQTogT2tcbkNSQS0-PkNSQTogU3RvcmUgSm9iIEhhbmRsZVxuQ1JBLT4-UzogSm9iSWRcblMtPj4tQ0E6IEpvYklkXG5KLT4-RlM6IFN0cmVhbSBvdXRwdXQgdG8gZmlsZSIsIm1lcm1haWQiOnsidGhlbWUiOiJkZWZhdWx0In0sInVwZGF0ZUVkaXRvciI6ZmFsc2UsImF1dG9TeW5jIjp0cnVlLCJ1cGRhdGVEaWFncmFtIjpmYWxzZX0)](https://mermaid.live/edit#eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBKIGFzIEpvYlxucGFydGljaXBhbnQgQ1JBIGFzIENvbnRyb2xsZXIgZm9yIENsaWVudCBBXG5wYXJ0aWNpcGFudCBGUyBhcyBGaWxlIFN0b3JhZ2VcblxuQ0EtPj4rUzogc3RhcnQ6IDxicj5jdXJsIGV4YW1wbGUuY29tIDxicj5jcHUud2VpZ2h0PTUwMFxuUy0-PkNSQTogbmV3IEpvYiByZXF1ZXN0IGZvciBDbGllbnQgQVxuQ1JBLT4-SjogbmV3KClcbkotPj5DUkE6IEpvYklkXG5DUkEtPj5DUkE6IHNldCB1cCBjZ3JvdXAgY29udHJvbHMgZm9yIEpvYklkXG5DUkEtPj5KOiBmb3JrKClcbkotPj5KOiBXcml0ZSBnZXRwaWQoKSB0byBjZ3JvdXAucHJvY3NcbkotPj5KOiBzZXRnaWQoKSBhbmQgc2V0dWlkKCkgdG8gQ2xpZW50IEEgdXNlclxuSi0-Pko6IGV4ZWMoKVxuSi0-PkNSQTogT2tcbkNSQS0-PkNSQTogU3RvcmUgSm9iIEhhbmRsZVxuQ1JBLT4-UzogSm9iSWRcblMtPj4tQ0E6IEpvYklkXG5KLT4-RlM6IFN0cmVhbSBvdXRwdXQgdG8gZmlsZSIsIm1lcm1haWQiOiJ7XG4gIFwidGhlbWVcIjogXCJkZWZhdWx0XCJcbn0iLCJ1cGRhdGVFZGl0b3IiOmZhbHNlLCJhdXRvU3luYyI6dHJ1ZSwidXBkYXRlRGlhZ3JhbSI6ZmFsc2V9)

Notes:

- [Command](https://docs.rs/tokio/1.14.0/tokio/process/struct.Command.html) has `pre_exec` method on it. I believe that for this use case leveraging `pre_exec` + `spawn` is semantically equivalent to using `fork()` and `exec()`.
- As mentioned earlier Job is an abstraction over child process which owns the handle and enables communication with other actors in the system. 

<details>
<summary> Start Job mermaid </summary>

```txt
sequenceDiagram
actor CA as Client A
participant S as Job Runner Service 
participant J as Job
participant CRA as Controller for Client A
participant FS as File Storage

CA->>+S: start: <br>curl example.com <br>cpu.weight=500
S->>CRA: new Job request for Client A
CRA->>J: new()
J->>CRA: JobId
CRA->>CRA: set up cgroup controls for JobId
CRA->>J: fork()
J->>J: Write getpid() to cgroup.procs
J->>J: setgid() and setuid() to Client A user
J->>J: exec()
J->>CRA: Ok
CRA->>CRA: Store Job Handle
CRA->>S: JobId
S->>-CA: JobId
J->>FS: Stream output to file 
```
</details>

### Status

[![](https://mermaid.ink/img/eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcblxuTm90ZSBvdmVyIENBLENSQTogSm9iIHdpdGggaWQgSm9iSWQgZm9yIENsaWVudCBBIHdhcyBzdGFydGVkIGVhcmxpZXJcbkNBLT4-UzogU3RhdHVzKEpvYklkKVxuUy0-PkNSQTogU3RhdHVzIGZvciBKb2JJZCBwbHNcbmFsdCBKb2Igc3RpbGwgcnVubmluZ1xuQ1JBLT4-UzogSWQgZXhpc3RzLCBubyBzdGF0dXMsIG11c3QgYmUgcnVubmluZ1xuUy0-PkNBOiBydW5uaW5nXG5lbHNlIEpvYiBleGl0ZWRcbkNSQS0-PlM6IElkIGV4aXN0cywgaGFzIGV4aXRfY29kZVxuUy0-PkNBOiBleGl0X2NvZGVcbmVsc2UgQ2xpZW50IGtpbGxzIEpvYlxuQ1JBLT4-UzogSWQgZXhpc3RzLCBoYXMgc2lnbmFsXG5TLT4-Q0E6IHNpZ25hbFxuZW5kIiwibWVybWFpZCI6eyJ0aGVtZSI6ImRlZmF1bHQifSwidXBkYXRlRWRpdG9yIjpmYWxzZSwiYXV0b1N5bmMiOnRydWUsInVwZGF0ZURpYWdyYW0iOmZhbHNlfQ)](https://mermaid.live/edit#eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcblxuTm90ZSBvdmVyIENBLENSQTogSm9iIHdpdGggaWQgSm9iSWQgZm9yIENsaWVudCBBIHdhcyBzdGFydGVkIGVhcmxpZXJcbkNBLT4-UzogU3RhdHVzKEpvYklkKVxuUy0-PkNSQTogU3RhdHVzIGZvciBKb2JJZCBwbHNcbmFsdCBKb2Igc3RpbGwgcnVubmluZ1xuQ1JBLT4-UzogSWQgZXhpc3RzLCBubyBzdGF0dXMsIG11c3QgYmUgcnVubmluZ1xuUy0-PkNBOiBydW5uaW5nXG5lbHNlIEpvYiBleGl0ZWRcbkNSQS0-PlM6IElkIGV4aXN0cywgaGFzIGV4aXRfY29kZVxuUy0-PkNBOiBleGl0X2NvZGVcbmVsc2UgQ2xpZW50IGtpbGxzIEpvYlxuQ1JBLT4-UzogSWQgZXhpc3RzLCBoYXMgc2lnbmFsXG5TLT4-Q0E6IHNpZ25hbFxuZW5kIiwibWVybWFpZCI6IntcbiAgXCJ0aGVtZVwiOiBcImRlZmF1bHRcIlxufSIsInVwZGF0ZUVkaXRvciI6ZmFsc2UsImF1dG9TeW5jIjp0cnVlLCJ1cGRhdGVEaWFncmFtIjpmYWxzZX0)

Notes:
- Status could be updated by Job task using `Arc<RwLock<JobOutcome>>`. Alternative approach I've considered was to use `Arc<AtomicU16>`, but that would require magic numbers. Given that value is updated once when the job is complete or terminated I believe `RwLock` is appropriate synchronization tool to use.  

<details>
<summary> Job status mermaid </summary>

```txt
sequenceDiagram
actor CA as Client A
participant S as Job Runner Service 
participant CRA as Controller for Client A

Note over CA,CRA: Job with id JobId for Client A was started earlier
CA->>S: Status(JobId)
S->>CRA: Status for JobId pls
alt Job still running
CRA->>S: Id exists, no status, must be running
S->>CA: running
else Job exited
CRA->>S: Id exists, has exit_code
S->>CA: exit_code
else Client kills Job
CRA->>S: Id exists, has signal
S->>CA: signal
end 
```
</details>

### Output

[![](https://mermaid.ink/img/eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcbnBhcnRpY2lwYW50IEogYXMgSm9iXG5wYXJ0aWNpcGFudCBGUyBhcyBGaWxlIFN5c3RlbVxuXG5Ob3RlIG92ZXIgQ0EsRlM6IEpvYiB3aXRoIGlkIEpvYklkIGZvciBDbGllbnQgQSB3YXMgc3RhcnRlZCBlYXJsaWVyXG5KLSlGUzogU3RyZWFtcyBsb2dzXG5DQS0-PlM6IE91dHB1dChKb2JJZClcblMtPj5DUkE6IE91dHB1dCBmb3IgSm9iSWQgcGxzXG5DUkEtPj5GUzogT3BlbiBMb2cgZmlsZXMgZm9yIEpvYklkXG5GUy0-PkNSQTogSGVyZSBpcyB0aGUgaGFuZGxlXG5DUkEtPj5TOiBTdHJlYW0gY29udGVudHMgYmFja1xuUy0-PkNBOiBIZXJlIGFyZSB5b3VyIGxvZ3MiLCJtZXJtYWlkIjp7InRoZW1lIjoiZGVmYXVsdCJ9LCJ1cGRhdGVFZGl0b3IiOmZhbHNlLCJhdXRvU3luYyI6dHJ1ZSwidXBkYXRlRGlhZ3JhbSI6ZmFsc2V9)](https://mermaid.live/edit#eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcbnBhcnRpY2lwYW50IEogYXMgSm9iXG5wYXJ0aWNpcGFudCBGUyBhcyBGaWxlIFN5c3RlbVxuXG5Ob3RlIG92ZXIgQ0EsRlM6IEpvYiB3aXRoIGlkIEpvYklkIGZvciBDbGllbnQgQSB3YXMgc3RhcnRlZCBlYXJsaWVyXG5KLSlGUzogU3RyZWFtcyBsb2dzXG5DQS0-PlM6IE91dHB1dChKb2JJZClcblMtPj5DUkE6IE91dHB1dCBmb3IgSm9iSWQgcGxzXG5DUkEtPj5GUzogT3BlbiBMb2cgZmlsZXMgZm9yIEpvYklkXG5GUy0-PkNSQTogSGVyZSBpcyB0aGUgaGFuZGxlXG5DUkEtPj5TOiBTdHJlYW0gY29udGVudHMgYmFja1xuUy0-PkNBOiBIZXJlIGFyZSB5b3VyIGxvZ3MiLCJtZXJtYWlkIjoie1xuICBcInRoZW1lXCI6IFwiZGVmYXVsdFwiXG59IiwidXBkYXRlRWRpdG9yIjpmYWxzZSwiYXV0b1N5bmMiOnRydWUsInVwZGF0ZURpYWdyYW0iOmZhbHNlfQ)

Notes:
- Considered keeping logs in-memory, but I believe this approach has couple of drawbacks. Reads and writes need to be synchronized somehow, so our options are either introducing Mutex, which may become fairly hot (depends on the volume of logs), or making Job own the output of the child process instead of streaming to file. This consequently means that all job output is now in main process memory, regardless of whether it will ever be read or not.  

<details>
<summary> Job output mermaid </summary>

```txt
sequenceDiagram
actor CA as Client A
participant S as Job Runner Service 
participant CRA as Controller for Client A
participant J as Job
participant FS as File System

Note over CA,FS: Job with id JobId for Client A was started earlier
J-)FS: Streams logs
CA->>S: Output(JobId)
S->>CRA: Output for JobId pls
CRA->>FS: Open Log files for JobId
FS->>CRA: Here is the handle
CRA->>S: Stream contents back
S->>CA: Here are your logs 
```
</details>

### Stop 

[![](https://mermaid.ink/img/eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcbnBhcnRpY2lwYW50IEogYXMgSm9iXG5wYXJ0aWNpcGFudCBGUyBhcyBGaWxlIFN5c3RlbVxuXG5Ob3RlIG92ZXIgQ0EsRlM6IEpvYiB3aXRoIGlkIEpvYklkIGZvciBDbGllbnQgQSB3YXMgc3RhcnRlZCBlYXJsaWVyXG5KLSlGUzogU3RyZWFtcyBsb2dzXG5DQS0-PlM6IFN0b3AoSm9iSWQpXG5TLT4-Q1JBOiBTdG9wIEpvYklkIHBsc1xuQ1JBLT4-SjogRWFybHkgb3V0XG5KLT4-RlM6IEZsdXNoIGxvZ3NcbkotPj5KOiBLaWxsIGNoaWxkIHByb2Nlc3NcbkotPj5DUkE6IERvbmVcbkNSQS0-PkNSQTogVXBkYXRlIHN0YXR1cyBmb3IgSm9iSWRcbkNSQS0-PlM6IERvbmVcblMtPj5DQTogQWNrIiwibWVybWFpZCI6eyJ0aGVtZSI6ImRlZmF1bHQifSwidXBkYXRlRWRpdG9yIjpmYWxzZSwiYXV0b1N5bmMiOnRydWUsInVwZGF0ZURpYWdyYW0iOmZhbHNlfQ)](https://mermaid.live/edit/#eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxucGFydGljaXBhbnQgUyBhcyBKb2IgUnVubmVyIFNlcnZpY2UgXG5wYXJ0aWNpcGFudCBDUkEgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEFcbnBhcnRpY2lwYW50IEogYXMgSm9iXG5wYXJ0aWNpcGudCBGUyBhcyBGaWxlIFN5c3RlbVxuXG5Ob3RlIG92ZXIgQ0EsRlM6IEpvYiB3aXRoIGlkIEpvYklkIGZvciBDbGllbnQgQSB3YXMgc3RhcnRlZCBlYXJsaWVyXG5KLSlGUzogU3RyZWFtcyBsb2dzXG5DQS0-PlM6IFN0b3AoSm9iSWQpXG5TLT4-Q1JBOiBTdG9wIEpvYklkIHBsc1xuQ1JBLT4-SjogRWFybHkgb3V0XG5KLT4-RlM6IEZsdXNoIGxvZ3NcbkotPj5KOiBLaWxsIGNoaWxkIHByb2Nlc3NcbkotPj5DUkE6IERvbmVcbkNSQS0-PkNSQTogVXBkYXRlIHN0YXR1cyBmb3IgSm9iSWRcbkNSQS0-PlM6IERvbmVcblMtPj5DQTogQWNrIiwibWVybWFpZCI6IntcbiAgXCJ0aGVtZVwiOiBcImRlZmF1bHRcIlxufSIsInVwZGF0ZUVkaXRvciI6ZmFsc2UsImF1dG9TeW5jIjp0cnVlLCJ1cGRhdGVEaWFncmFtIjpmYWxzZX0)

<details>
<summary> Stop Job mermaid </summary>


```txt
sequenceDiagram
actor CA as Client A
participant S as Job Runner Service 
participant CRA as Controller for Client A
participant J as Job
participant FS as File System

Note over CA,FS: Job with id JobId for Client A was started earlier
J-)FS: Streams logs
CA->>S: Stop(JobId)
S->>CRA: Stop JobId pls
CRA->>J: Early out
J->>FS: Flush logs
J->>J: Kill child process
J->>CRA: Done
CRA->>CRA: Update status for JobId
CRA->>S: Done
S->>CA: Ack
```
</details>


### Authz 

[![](https://mermaid.ink/img/eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxuYWN0b3IgQ0IgYXMgQ2xpZW50IEJcbnBhcnRpY2lwYW50IFMgYXMgSm9iIFJ1bm5lciBTZXJ2aWNlIFxucGFydGljaXBhbnQgQ1JBIGFzIENvbnRyb2xsZXIgZm9yIENsaWVudCBBXG5wYXJ0aWNpcGFudCBDUkIgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEJcblxuTm90ZSBvdmVyIENBLCBDQjogQ2xpZW50IEIgZ2V0cyBKb2JJZCBvZiBDbGllbnQgQSBzb21laG93XG5DQi0-PlM6IE91dHB1dChKb2JJZClcblMtPj5DUkI6IE91dHB1dCBmb3IgSm9iSWQgcGxzXG5DUkItPj5TOiBObyBzdWNoIEpvYlxuUy0-PkNCOiBOb3QgZm91bmQiLCJtZXJtYWlkIjp7InRoZW1lIjoiZGVmYXVsdCJ9LCJ1cGRhdGVFZGl0b3IiOmZhbHNlLCJhdXRvU3luYyI6dHJ1ZSwidXBkYXRlRGlhZ3JhbSI6ZmFsc2V9)](https://mermaid.live/edit#eyJjb2RlIjoic2VxdWVuY2VEaWFncmFtXG5hY3RvciBDQSBhcyBDbGllbnQgQVxuYWN0b3IgQ0IgYXMgQ2xpZW50IEJcbnBhcnRpY2lwYW50IFMgYXMgSm9iIFJ1bm5lciBTZXJ2aWNlIFxucGFydGljaXBhbnQgQ1JBIGFzIENvbnRyb2xsZXIgZm9yIENsaWVudCBBXG5wYXJ0aWNpcGFudCBDUkIgYXMgQ29udHJvbGxlciBmb3IgQ2xpZW50IEJcblxuTm90ZSBvdmVyIENBLCBDQjogQ2xpZW50IEIgZ2V0cyBKb2JJZCBvZiBDbGllbnQgQSBzb21laG93XG5DQi0-PlM6IE91dHB1dChKb2JJZClcblMtPj5DUkI6IE91dHB1dCBmb3IgSm9iSWQgcGxzXG5DUkItPj5TOiBObyBzdWNoIEpvYlxuUy0-PkNCOiBOb3QgZm91bmQiLCJtZXJtYWlkIjoie1xuICBcInRoZW1lXCI6IFwiZGVmYXVsdFwiXG59IiwidXBkYXRlRWRpdG9yIjpmYWxzZSwiYXV0b1N5bmMiOnRydWUsInVwZGF0ZURpYWdyYW0iOmZhbHNlfQ)

<details>
<summary> Authz mermaid </summary>

```txt
sequenceDiagram
actor CA as Client A
actor CB as Client B
participant S as Job Runner Service 
participant CRA as Controller for Client A
participant CRB as Controller for Client B

Note over CA, CB: Client B gets JobId of Client A somehow
CB->>S: Output(JobId)
S->>CRB: Output for JobId pls
CRB->>S: No such Job
S->>CB: Not not-found
```
</details>


## CLI 

For cli, aiming to use [structopt](https://github.com/TeXitoi/structopt) which is a great tool for parsing cli arguments. Api could look somewhat like this: 

```
<bin> start -e curl -a example.com -cpu-weight 250 -memory-max 3145728 -u localhost:6666 -f cert
// => JobId: abc-xyz
<bin> status abc-xyz -u localhost:6666 -f cert
// => exit_code 0
```

Typing params that do not change from request to request gets very old very fast, in production environment presence of config file with default profile in well-known location (`~/.config/.runner.json`) could be used instead.  

## High Availability

In its current state service runs on a single host. To move job runner into the world of high availability, we would need to introduce some indirection. 
One way to go about it is to add routing layer which has the knowledge of which controllers run on what machines, and consequently which jobs are executed on which machines. That way, requests of particular client could be forwarded to a subgroup of hosts.  
This introduces a whole set of interesting challenges such as state checkpoints (in case of required process migrations), and further design decisions depend heavily on the business problem clients are trying to solve. 
