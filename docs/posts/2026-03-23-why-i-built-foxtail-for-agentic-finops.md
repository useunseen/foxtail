# Why I Built Foxtail for Agentic FinOps

I ran into Foxtail because LocalStack solved only half of the problem I had.

For an Agentic FinOps product, LocalStack is useful right away. I can stand up AWS-like resources, poke at them, and build local workflows around them. But after that first bit of progress, I kept hitting the same wall: the resources existed, but the FinOps evidence did not.

That matters because a FinOps agent is not just looking for inventory. It needs cost data, usage trends, CloudWatch metrics, pricing lookups, recommendation surfaces, tagged inventory, the kinds of things a human would pull when trying to explain spend or find waste.

I had a local AWS-shaped environment. I did not have a local AWS-shaped FinOps environment.

My first version went in the direction most people go. I created static mocked metrics data and hid it behind an abstraction layer so the rest of the app would not care where the numbers came from.

It was fine for a minute. It got the UI moving. It let me demo some flows. But it started to feel wrong pretty quickly.

The issue was not that the numbers were fake. Mock data is expected in local development. The issue was that I was teaching the system to depend on a fake interface that had nothing to do with how the real product would eventually work.

Those mocked payloads were not tied to the resources I had deployed. The agent was not learning how to discover cost or metric evidence through AWS-like workflows. It was just getting fed answers in a shape I invented because it was convenient.

That is when the whole thing clicked for me. I did not want to mock the data my app happened to need at that moment. I wanted to mock the APIs my agent should eventually rely on.

That pushed me toward a very different standard. Instead of calling some internal helper for fake metrics, I wanted the local loop to look like this:

```bash
aws ce get-cost-and-usage ...
aws cloudwatch list-metrics ...
aws pricing get-products ...
```

Once I had that in mind, the implementation direction became much clearer.

Foxtail is a local Rust service that serves AWS-like FinOps data from SQLite. The key thing it does is sit next to LocalStack instead of replacing it. LocalStack still handles the resource side. Foxtail handles the FinOps side.

The loop now looks like this:

1. Deploy AWS-like resources into LocalStack.
2. Let Foxtail discover those resources.
3. Seed a scenario like baseline, spike, or idle-heavy.
4. Start the Foxtail service.
5. Query it through the AWS CLI.

That is a much healthier setup for the agent. The data is still synthetic, but it is no longer arbitrary. It is derived from the local environment and shaped into scenarios that are actually useful for FinOps work.

Foxtail covers the surfaces I care about most in that workflow: Cost Explorer-style calls, CloudWatch metric discovery and queries, Resource Groups Tagging API lookups, pricing lookups, Compute Optimizer-style recommendations, and CUR report definition discovery. I am not trying to emulate all of AWS. I just want enough of the FinOps-facing surface area that an agent can investigate cost, usage, waste, and optimization opportunities without having to learn a fake local-only protocol.

There was one more problem to solve once Foxtail existed.

The AWS CLI only gets one `--endpoint-url` at a time, which is annoying when you have two local services playing different roles:

- infrastructure-style commands should still go to LocalStack, or to real AWS
- FinOps-style commands should go to Foxtail

I did not want to push that complexity into the agent. The agent should not have to remember endpoint rules or branch on whether a command is “FinOps enough” to hit one service versus another.

So I added a wrapper CLI called `foxtail`. It does one job:

- if the command is FinOps-related and Foxtail supports it, route it to the Foxtail service
- otherwise, route it to LocalStack or AWS as normal

That lets the agent treat the CLI like one surface:

```bash
foxtail ce get-cost-and-usage ...
foxtail cloudwatch list-metrics ...
foxtail s3 ls
```

Under the hood it can still route to different places, but that routing is environmental plumbing, not agent logic.

That distinction matters more than it sounds. Every time I can keep setup weirdness out of the agent’s prompt and tools, I get something more robust.

The place where Foxtail helps most is the messy middle: too early for real AWS billing and observability data, but too late for static mock JSON to be useful. That covers a lot of practical cases. Early product development is one. Demos are another. Repeatable test scenarios are another. If I want to force a spike or an idle-heavy environment and see how the agent reasons about it, Foxtail gives me a clean place to do that.

It is not the right answer for everything. If I only needed a few screenshots or a simple UI prototype, I would not bother with this setup. And it definitely does not remove the need to test against real AWS later. It is a bridge, not a substitute.

What I like about Foxtail is that it keeps the abstraction honest. The agent is still learning to discover resources, pull metrics, compare usage to spend, inspect tags, and form a view through AWS CLI-compatible calls. That is much closer to the real job than handing it a custom local endpoint full of pre-shaped answers.

That is really why I built it.

I wanted LocalStack to keep doing what it is already good at. I wanted a separate layer for FinOps evidence. And I wanted the agent to operate through one CLI-shaped interface without caring which backend handled which command.

For me, that has been a much better starting point than inventing a private mock metrics API and hoping I could unwind it later.
