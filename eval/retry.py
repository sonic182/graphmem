"""Reuse Strands backoff for LiteLLM's unconverted rate-limit errors."""

from litellm import RateLimitError
from strands import ModelRetryStrategy


class LiteLLMRetryStrategy(ModelRetryStrategy):
    """Retry provider rate limits as well as native Strands throttling."""

    def is_retryable(self, exception: Exception) -> bool:
        return isinstance(exception, RateLimitError) or super().is_retryable(exception)
