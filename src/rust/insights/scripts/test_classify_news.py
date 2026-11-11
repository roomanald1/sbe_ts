import asyncio
import json
import unittest
from unittest.mock import patch

import classify_news
from rsocket.payload import Payload


class NewsSentimentRSocketTests(unittest.IsolatedAsyncioTestCase):
    async def test_request_response_returns_json_prediction_payload(self):
        expected = [{"direction": "bullish", "score": 0.7, "confidence": 0.9}]
        handler = classify_news.SentimentRequestHandler()
        with patch.object(classify_news, "classify", return_value=expected):
            future = await handler.request_response(
                Payload(data=json.dumps([{"title": "BTC adoption grows"}]).encode())
            )
            response = await future
        self.assertEqual(json.loads(response.data), expected)

    async def test_request_validation_rejects_invalid_input(self):
        with self.assertRaises(ValueError):
            classify_news.decode_request(b'{"title":"not an array"}')

        with self.assertRaises(ValueError):
            classify_news.decode_request(b'[{"article_text":"missing title"}]')

    async def test_request_limit_is_enforced(self):
        with self.assertRaises(ValueError):
            classify_news.decode_request(b" " * 2_000_001)


if __name__ == "__main__":
    unittest.main()
