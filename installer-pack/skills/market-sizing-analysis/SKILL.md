---
name: market-sizing-analysis
description: This skill should be used when the user asks to "calculate TAM", "determine SAM", "estimate SOM", "size the market", "calculate market opportunity", "what's the total addressable market", or requests market sizing analysis for a startup or business opportunity.
---

# Market Sizing Analysis

Comprehensive market sizing methodologies for calculating Total Addressable Market (TAM), Serviceable Available Market (SAM), and Serviceable Obtainable Market (SOM) for startup opportunities.

## Overview

Market sizing provides the foundation for startup strategy, fundraising, and business planning. Calculate market opportunity using three complementary methodologies: top-down (industry reports), bottom-up (customer segment calculations), and value theory (willingness to pay).

## Core Concepts

### The Three-Tier Market Framework

**TAM (Total Addressable Market)**

- Total revenue opportunity if achieving 100% market share
- Defines the universe of potential customers
- Used for long-term vision and market validation
- Example: All email marketing software revenue globally

**SAM (Serviceable Available Market)**

- Portion of TAM targetable with current product/service
- Accounts for geographic, segment, or capability constraints
- Represents realistic addressable opportunity
- Example: AI-powered email marketing for e-commerce in North America

**SOM (Serviceable Obtainable Market)**

- Revenue or customer volume plausibly reachable within a stated horizon, based on explicit capacity and adoption assumptions
- Accounts for competition, resources, and market dynamics
- Used for financial projections and fundraising
- Do not use a universal SOM percentage. Derive scenarios from sales capacity, conversion, retention, competition, and time horizon; label unsupported inputs as assumptions.

### When to Use Each Methodology

**Top-Down Analysis**

- Use when established market research exists
- Best for mature, well-defined markets
- Validates market existence and growth
- Starts with industry reports and narrows down

**Bottom-Up Analysis**

- Use when targeting specific customer segments
- Best for new or niche markets
- Can be granular and defensible when customer data and assumptions are supported
- Builds from customer data and pricing

**Value Theory**

- Use when creating new market categories
- Best for disruptive innovations
- Estimates based on value creation
- Calculates willingness to pay for problem solution

## Three-Methodology Framework

### Methodology 1: Top-Down Analysis

Start with total market size and narrow to addressable segments.

**Process:**

1. Identify total market category from research reports
2. Apply geographic filters (target regions)
3. Apply segment filters (target industries/customers)
4. Apply explicit, evidenced segment filters; do not infer addressable customer counts from market-share claims alone.

**Formula:**

```
TAM = Total Market Category Size
SAM = TAM × Joint Share Meeting All Geographic and Segment Filters
SOM customers = min(serviceable customers, reachable customers under explicit sales/adoption/delivery capacity over the horizon)
SOM revenue = SOM customers × annual revenue per customer
```

**When to use:** Established markets with available research (e.g., SaaS, fintech, e-commerce)

**Strengths:** Can provide a quick market context when the underlying source and definition are credible and documented

**Limitations:** May overestimate for new categories, less granular

### Methodology 2: Bottom-Up Analysis

Build market size from customer segment calculations.

**Process:**

1. Define target customer segments
2. Estimate number of potential customers per segment
3. Determine average revenue per customer
4. Calculate realistic penetration rates

**Formula:**

```
TAM = Σ (Eligible Customers in Each Mutually Exclusive Segment × Annual Revenue per Customer)
SAM = Σ (Eligible Customers in Each Serviceable, Mutually Exclusive Segment × Annual Revenue per Customer)
SOM = min(serviceable customers, customers reachable at modeled go-to-market capacity) × annual revenue per customer
```

Sum eligible revenue across the serviceable segments; do not scale TAM by the count of segments, because segments may differ greatly in customer count and revenue. For top-down estimates that only provide aggregate TAM, use a directly evidenced share that satisfies **all** filters. If applying filters sequentially, each later percentage must be conditional on the population remaining after the earlier filter. Do not multiply independent-looking geographic and segment percentages unless the joint share is evidenced or the independence assumption is stated and supported.

**Synthetic teaching example:** Two mutually exclusive segments have annual revenues of $99 and $1, for TAM $100. If only the $99 segment is serviceable, SAM is $99; if only the $1 segment is serviceable, SAM is $1. Counting segments would incorrectly assign $50 to either case.

**When to use:** B2B, niche markets, specific customer segments

**Strengths:** Granular and auditable when customer counts, pricing, and segment definitions are evidenced

**Limitations:** Requires detailed customer research, time-intensive

### Methodology 3: Value Theory

Calculate based on value created and willingness to pay.

**Process:**

1. Identify problem being solved
2. Quantify current cost of problem (time, money, inefficiency)
3. Calculate value of solution (savings, gains, efficiency)
4. Model willingness to pay with customer evidence or clearly labeled sensitivity scenarios; there is no universal percentage.
5. Multiply by addressable customer base

**Formula:**

```
Value per Customer = Problem Cost × % Solved by Solution
Price per Customer = Value × Assumed or observed willingness-to-pay rate
TAM = Total Potential Customers × Price per Customer
SAM = TAM × Share of TAM Meeting All Buy and Serviceability Criteria
SOM = reachable customers/revenue from an explicit capacity and adoption model
```

**When to use:** New categories, disruptive innovations, unclear existing markets

**Strengths:** Shows value creation, works for new markets

**Limitations:** Requires assumptions, harder to validate

## Step-by-Step Process

### Step 1: Define the Market

Clearly specify what market is being measured.

**Questions to answer:**

- What problem is being solved?
- Who are the target customers?
- What's the product/service category?
- What's the geographic scope?
- What's the time horizon?

**Example:**

- Problem: E-commerce companies struggle with email marketing automation
- Customers: a clearly defined subset of e-commerce stores
- Category: AI-powered email marketing software
- Geography: North America initially, global expansion
- Horizon: a stated planning period selected for the analysis

This is a scope illustration, not a market fact or recommended customer threshold.

### Step 2: Gather Data Sources

Use the current Codex session's native web search/browser when available. Open primary sources, capture publication dates and units,
and keep assumptions separate from observed data. If web access is unavailable, use only user-provided sources and state the verification gap.

Identify credible data for calculations.

For every decision-critical input, record the source URL or user-provided origin, publication date, reference period, unit, definition, geography, and access/verification status. Prefer primary data and independently corroborate material estimates where possible; syndicated copies of the same estimate are not independent sources. Reconcile different definitions as ranges instead of averaging unlike figures. Label each value as observed fact, user-provided data, assumption, or calculation. A broad sector revenue figure does not validate a niche customer count. Never invent citations, report titles, survey results, market shares, or current prices. If a source cannot be opened, say so and treat its claim as unverified.

**Top-Down Sources:**

- Industry research reports (Gartner, Forrester, IDC), subject to access rights
- Government statistics (Census, BLS, trade associations)
- Public company filings and earnings
- Market research firms (Statista, CB Insights, PitchBook), subject to access rights

**Bottom-Up Sources:**

- Customer interviews and surveys
- Sales data and CRM records
- Industry databases (LinkedIn, ZoomInfo, Crunchbase), subject to access rights
- Competitive intelligence
- Academic research

**Value Theory Sources:**

- Customer problem quantification
- Time/cost studies
- ROI case studies
- Pricing research and willingness-to-pay surveys

### Step 3: Calculate TAM

Apply chosen methodology to determine total market.

**For Top-Down:**

1. Find total category size from research
2. Document data source and year
3. Apply growth rate if needed
4. Validate with multiple sources

**For Bottom-Up:**

1. Count total potential customers
2. Calculate average annual revenue per customer
3. Multiply to get TAM
4. Break down by segment

**For Value Theory:**

1. Quantify total addressable customer base
2. Calculate value per customer
3. Estimate pricing based on value
4. Multiply for TAM

### Step 4: Calculate SAM

Narrow TAM to serviceable addressable market.

**Apply Filters:**

- Geographic constraints (regions you can serve)
- Product limitations (features you currently have)
- Customer requirements (size, industry, use case)
- Distribution channel access
- Regulatory or compliance restrictions

**Formula:**

```
SAM = TAM × (% matching all filters)
```

**Example:** Apply documented geographic, product, customer, and regulatory filters to sourced customer/revenue data. Do not use unverified percentages to turn a broad category total into a precise SAM.

### Step 5: Calculate SOM

Estimate reachable customer count and revenue under a stated horizon.

**Consider:**

- Current market share of competitors
- Sales capacity, conversion, retention, pricing, competition, and time horizon
- Resources available (funding, team, time)
- Go-to-market effectiveness
- Competitive advantages
- A user-defined planning horizon; no standard duration applies to every market

**Capacity-based scenarios:**

```
Year-N customers = qualified opportunities × conversion, capped by delivery and sales capacity
Year-N SOM revenue = reachable active customers × annual revenue per customer
```

Report one or more time-based scenarios only when the underlying customer and capacity inputs are documented.

### Step 6: Validate and Triangulate

Cross-check using multiple methods.

**Validation Techniques:**

1. Compare top-down and bottom-up results; explain differences in definitions, periods, and source quality rather than enforcing a universal tolerance.
2. Check against public company revenues in space
3. Validate customer count assumptions
4. Sense-check pricing assumptions
5. Review with industry experts
6. Compare to similar market categories

**Red Flags:**

- TAM that does not meet the explicitly stated business or financing criteria; there is no universal minimum threshold
- TAM that's too large (unsupported by data)
- SOM that exceeds modeled sales, delivery, or customer adoption capacity
- Material disagreement between methods that is left unexplained

## Industry-Specific Considerations

### SaaS Markets

**Key Metrics:**

- Number of potential businesses in target segment
- Average contract value (ACV)
- Typical market penetration rates
- Expansion revenue potential

**TAM Calculation:**

```
TAM = Total Target Companies × Average ACV × (1 + Expansion Rate)
```

### Marketplace Markets

**Key Metrics:**

- Gross Merchandise Value (GMV) of category
- Take rate (% of GMV you capture)
- Total transactions or users

**TAM Calculation:**

```
TAM = Total Category GMV × Expected Take Rate
```

### Consumer Markets

**Key Metrics:**

- Total addressable users/households
- Average revenue per user (ARPU)
- Engagement frequency

**TAM Calculation:**

If ARPU is annual:

`TAM = Total Addressable Users × Annual ARPU`

If revenue is measured per purchase instead:

`TAM = Total Addressable Users × Average Spend per Purchase × Purchases per User per Year`

Do not multiply annual ARPU by purchase frequency: annual ARPU already includes the year's purchases. State the ARPU period and use a matching 12-month frequency when using per-purchase spend.

**Synthetic teaching example:** 10,000 users × $100 annual ARPU = $1,000,000 TAM. If $100 is average spend per purchase and users purchase twice per year, annual TAM is $2,000,000.

### B2B Services

**Key Metrics:**

- Number of target companies by size/industry
- Average project value or retainer
- Typical buying frequency

**TAM Calculation:**

```
TAM = Total Target Companies × Average Deal Size × Deals per Year
```

## Presenting Market Sizing

### For Investors

**Structure:**

1. Market definition and problem scope
2. TAM/SAM/SOM with methodology
3. Data sources and assumptions
4. Growth projections and drivers
5. Competitive landscape context

**Key Points:**

- Select the method whose inputs and definitions can be evidenced; explain its limits
- Triangulate with other methods when comparable data is available
- Explain conservative assumptions
- Link to revenue projections
- Highlight market growth rate

### For Strategy

**Structure:**

1. Addressable customer segments
2. Prioritization by opportunity size
3. Entry strategy by segment
4. Expected penetration timeline
5. Resource requirements

**Key Points:**

- Focus on SAM and SOM
- Show segment-level detail
- Connect to go-to-market plan
- Identify expansion opportunities
- Discuss competitive positioning

## Common Mistakes to Avoid

**Mistake 1: Confusing TAM with SAM**

- Don't claim entire market as addressable
- Apply realistic product/geographic constraints
- Be honest about serviceable market

**Mistake 2: Overly Aggressive SOM**

- Do not apply a generic market-share ceiling; show the capacity assumptions and uncertainty instead.
- Account for competition and resources
- Show realistic ramp timeline

**Mistake 3: Using Only Top-Down**

- Top-down results alone may be too coarse for an operating plan
- Use a second method when comparable evidence is available; explain when it is not

**Mistake 4: Cherry-Picking Data**

- Use consistent, recent data sources
- Don't mix methodologies inappropriately
- Document all assumptions clearly

**Mistake 5: Ignoring Market Dynamics**

- Account for market growth/decline
- Consider competitive intensity
- Factor in switching costs and barriers

## Additional Resources

### Reference Files

For detailed methodologies and frameworks:

- **`references/methodology-deep-dive.md`** - Comprehensive guide to each methodology with step-by-step worksheets
- **`references/data-sources.md`** - Curated list of market research sources, databases, and tools
- **`references/industry-templates.md`** - Specific templates for SaaS, marketplace, consumer, B2B, and fintech markets

### Example Files

Working examples with complete calculations:

- **`examples/saas-market-sizing.md`** - Complete TAM/SAM/SOM for a B2B SaaS product
- **`examples/marketplace-sizing.md`** - Marketplace platform market opportunity calculation
- **`examples/value-theory-example.md`** - Value-based market sizing for disruptive innovation

Use these examples as templates. All included numbers are explicitly hypothetical and demonstrate arithmetic only; they are not market facts or validated benchmarks. Replace them with sourced inputs before making decisions.

## Quick Start

To perform market sizing analysis:

1. **Define the market** - Problem, customers, category, geography
2. **Choose methodology** - Select the method supported by the available evidence; triangulate when definitions can be aligned
3. **Gather data** - Industry reports, customer data, competitive intelligence
4. **Calculate TAM** - Apply methodology formula
5. **Narrow to SAM** - Apply product, geographic, segment filters
6. **Estimate SOM** - model reachable customers/revenue from explicit capacity and adoption assumptions
7. **Validate** - Cross-check with alternative methods
8. **Document** - Show methodology, sources, assumptions
9. **Present** - Structure for audience (investors, strategy, operations)

For detailed step-by-step guidance on each methodology, reference the files in `references/` directory. For complete worked examples, see `examples/` directory.
