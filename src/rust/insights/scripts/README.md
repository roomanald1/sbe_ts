# Local news sentiment

The backend runner starts a local HTTP service for the crypto sentiment classifier burakutf/finetuned-finbert-crypto before starting Insights. The service loads the model once, listens on 127.0.0.1:3006, and is stopped with the rest of the backend services. Insights analyzes each headline with the short description supplied in the RSS item, when present. It does not scrape CoinDesk article pages. The widget shows that RSS context in an expandable section.

For a fresh workspace, install CPU-only PyTorch and the classifier packages into src/rust/insights/.news-python-packages. First install torch using the PyTorch CPU wheel index, then install packages listed in requirements-news-sentiment.txt with pip --target set to the same directory. run-backend-services.sh configures PYTHONPATH and HF_HOME automatically. Set NEWS_SENTIMENT_PYTHON to select a Python executable, or NEWS_SENTIMENT_URL to change the endpoint used by Insights.

This workspace already has those dependencies installed locally and the model weights cached under src/rust/insights/.news-model-cache. If inference fails, Insights retains the keyword-based fallback. The model card declares MIT licensing: https://huggingface.co/burakutf/finetuned-finbert-crypto. Confidence measures text classification confidence, not probability of price movement. The learned market-score weight remains separately capped at 20%.

CoinDesk's terms prohibit automated scraping of its site without express written permission, and its media kit describes full-text RSS as licensed use. This integration therefore uses the feed-provided description only; full article body retrieval needs permission from CoinDesk.
