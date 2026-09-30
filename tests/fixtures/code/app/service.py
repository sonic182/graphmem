import os


class BillingService:
    def charge(self, amount):
        return amount


def load_config(path):
    return os.path.exists(path)

from pathlib import Path
