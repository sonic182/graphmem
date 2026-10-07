"""Reserve a shared estimated-cost budget before every model request."""

import json
import threading
from decimal import Decimal

from litellm import AuthenticationError, BadRequestError, RateLimitError
from strands.hooks import AfterModelCallEvent, BeforeModelCallEvent, HookProvider, HookRegistry
from strands.types.exceptions import ModelThrottledException


class EvaluationLimitError(RuntimeError):
    """Stop an agent without retrying or discarding previously paid usage."""


class MatrixBudget:
    """Share one cost ceiling across all concurrent agents in a matrix."""

    def __init__(self, max_cost_usd: float, rates: dict[str, float]) -> None:
        self.limit = Decimal(str(max_cost_usd))
        if not self.limit.is_finite() or self.limit <= 0:
            raise ValueError("max-cost-usd must be finite and positive")
        self.rates = {key: Decimal(str(value)) for key, value in rates.items()}
        for key in ("input", "output"):
            if key not in self.rates or not self.rates[key].is_finite() or self.rates[key] <= 0:
                raise ValueError(f"missing or invalid {key} price")
        self.rates.setdefault("cache_read", self.rates["input"])
        if (
            not self.rates["cache_read"].is_finite()
            or not 0 <= self.rates["cache_read"] <= self.rates["input"]
        ):
            raise ValueError("invalid cache_read price")
        self.spent = Decimal(0)
        self.reserved = Decimal(0)
        self.untrusted = False
        self._condition = threading.Condition()

    def price(self, inputs: int, outputs: int, cached: int = 0) -> Decimal:
        """Calculate estimated USD from the configured per-million rates."""
        cached = min(max(cached, 0), inputs)
        return (
            (inputs - cached) * self.rates["input"]
            + cached * self.rates["cache_read"]
            + outputs * self.rates["output"]
        ) / 1_000_000

    def reserve(self, amount: Decimal) -> None:
        """Wait for pending calls to settle, or reject an unaffordable request."""
        with self._condition:
            while not self.untrusted and self.spent + self.reserved + amount > self.limit:
                if not self.reserved or amount > self.limit:
                    raise EvaluationLimitError("matrix cost budget cannot fund the next call")
                self._condition.wait()
            if self.untrusted:
                raise EvaluationLimitError("provider usage exceeded its cost reservation")
            self.reserved += amount

    def settle(self, reservation: Decimal, actual: Decimal) -> None:
        """Release unused funds; prevent further calls if estimation was unsafe."""
        with self._condition:
            self.reserved -= reservation
            self.spent += actual
            self.untrusted |= actual > reservation
            self._condition.notify_all()


class RunBudget(HookProvider):
    """Bound one agent's tokens and reserve funds for each retry attempt."""

    def __init__(self, matrix: MatrixBudget, max_tokens: int, generation_cap: int) -> None:
        if max_tokens <= 0 or generation_cap <= 0:
            raise ValueError("token limits must be positive")
        self.matrix = matrix
        self.max_tokens = max_tokens
        self.generation_cap = generation_cap
        self.tokens = 0
        self.cost = Decimal(0)
        self.reservation: Decimal | None = None
        self.output_cap = generation_cap
        self.stop_reason: str | None = None
        self.usage_unknown = False

    def register_hooks(self, registry: HookRegistry) -> None:
        """Intercept every model attempt, including SDK retries."""
        registry.add_callback(BeforeModelCallEvent, self.before_call)
        registry.add_callback(AfterModelCallEvent, self.after_call)

    def close(self) -> None:
        """Charge an abandoned request's reservation instead of stranding waiters."""
        if self.reservation is not None:
            reservation, self.reservation = self.reservation, None
            self.cost += reservation
            self.usage_unknown = True
            self.matrix.settle(reservation, reservation)

    def before_call(self, event: BeforeModelCallEvent) -> None:
        """Clamp output to the remaining token allowance and reserve cash."""
        if self.stop_reason:
            raise EvaluationLimitError(self.stop_reason)
        projected = event.projected_input_tokens
        if projected is None or projected < 0:
            self.stop_reason = "input token estimate unavailable"
            raise EvaluationLimitError(self.stop_reason)
        output_cap = min(self.generation_cap, self.max_tokens - self.tokens - projected)
        if output_cap <= 0:
            self.stop_reason = "run token budget cannot fund the next call"
            raise EvaluationLimitError(self.stop_reason)
        params = dict(event.agent.model.get_config().get("params") or {})
        event.agent.model.update_config(params=params | {"max_tokens": output_cap})

        # Reserve fresh-input cost, never assume a cache hit. One token per
        # serialized UTF-8 byte plus framing allowance is deliberately much
        # more conservative than the SDK's tokenizer estimate. This protects
        # cash even when that estimate is inaccurate for a new provider.
        payload = json.dumps(
            {
                "messages": event.agent.messages,
                "system": event.agent.system_prompt,
                "tools": event.agent.tool_registry.get_all_tool_specs(),
            },
            ensure_ascii=True,
        )
        input_bound = max(projected, len(payload.encode()) + 4096)
        reservation = self.matrix.price(input_bound, output_cap)
        try:
            self.matrix.reserve(reservation)
        except EvaluationLimitError as error:
            self.stop_reason = str(error)
            raise
        self.reservation = reservation
        self.output_cap = output_cap

    def after_call(self, event: AfterModelCallEvent) -> None:
        """Settle delivered usage before another concurrent agent can spend it."""
        if self.reservation is None:
            return
        reservation, self.reservation = self.reservation, None
        response = event.stop_response
        usage = response.message.get("metadata", {}).get("usage", {}) if response else {}
        if (
            isinstance(usage.get("inputTokens"), int)
            and usage["inputTokens"] > 0
            and isinstance(usage.get("outputTokens"), int)
            and usage["outputTokens"] >= 0
        ):
            cached = usage.get("cacheReadInputTokens", 0)
            actual = self.matrix.price(
                usage["inputTokens"],
                usage["outputTokens"],
                cached if isinstance(cached, int) else 0,
            )
            self.tokens += usage["inputTokens"] + usage["outputTokens"]
        elif isinstance(
            event.exception,
            (RateLimitError, ModelThrottledException, AuthenticationError, BadRequestError),
        ):
            actual = Decimal(0)  # Rejected requests; no delivered generation.
        else:
            actual = reservation  # Unknown usage is not permission to spend it again.
            self.usage_unknown = True
            self.stop_reason = "provider token usage unavailable"
        self.cost += actual
        self.matrix.settle(reservation, actual)
        if actual > reservation:
            self.stop_reason = "provider usage exceeded its cost reservation"
        elif self.tokens > self.max_tokens:
            self.stop_reason = "provider usage exceeded run token budget"
        elif (
            response
            and response.stop_reason == "max_tokens"
            and self.output_cap < self.generation_cap
        ):
            self.stop_reason = "run token allowance limited this generation"
        # Do not raise here: the SDK has not yet accumulated this paid usage.
        # Reject the next call, or let run_once mark a terminal overrun as an error.
