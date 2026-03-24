# Why I Built Foxtail for Agentic FinOps

I built Foxtail because I kept running into the same problem while working on an agentic FinOps product:

LocalStack gave me the infrastructure shape of AWS, but not the evidence a FinOps agent actually needs to reason well.

I could spin up resources locally. That part was fine.

What I could not do was ask useful questions about spend, usage, waste, pricing, tags, or optimization opportunities in a way that felt close to the real job.

So I tried the obvious shortcut first. I mocked the data.

It worked for a minute. The UI moved. The demos looked fine. But the deeper I got, the more it bothered me.

I was not teaching the system how to investigate FinOps questions through AWS-like workflows. I was feeding it answers through a fake interface I had invented just to keep moving.

That usually creates debt you can feel before you can name it.

The thing that finally clicked for me was this:

I did not want to mock the app's current needs.
I wanted to mock the APIs the agent should eventually rely on.

That led to Foxtail.

Foxtail is a local service that sits next to LocalStack. LocalStack handles the resource side. Foxtail handles the FinOps side: cost and usage, metrics, pricing, tagging, recommendations, and the other evidence an agent needs to investigate cloud spend properly.

What I wanted was a local loop that still felt honest.

Not:
some internal helper returns fake metrics

But:
the agent asks for cost data, usage data, tags, and pricing through CLI-compatible workflows that resemble the real thing

That distinction matters a lot to me.

I think one of the easiest ways to weaken an agentic product is to make the local environment too convenient in ways production never will be. You get fast progress, but you train against the wrong abstractions.

Foxtail is my attempt to avoid that.

It is not a replacement for real AWS.
It is not meant to emulate everything.
It is just a better bridge between static mock data and real cloud environments.

And honestly, that messy middle is where a lot of product building happens.

That is why I built it.
