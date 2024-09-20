#!/usr/bin/env python3

import requests
import getpass
import json
import os
import sys
import concurrent.futures
import logging
import typer
from keycloak import KeycloakOpenID

# Constants
APPLICATION_NAME: str = "Drop4Crop"
DROP4CROP_SERVER: str = "https://drop4crop-dev.epfl.ch"
TOKEN_CACHE_FILE: str = "token_cache.json"
UPLOAD_ENDPOINT: str = "/api/layers/uploads"
DEFAULT_THREADS: int = 10
OVERWRITE_DUPLICATES: bool = True

# The various items used to match in the directory structure
CROP_ITEMS = [
    "barley",
    "maize",
    "potato",
    "rice",
    "sorghum",
    "soy",
    "sugarcane",
    "wheat",
]
CROP_SPECIFIC_VARIABLES = [
    "mirca_area_irrigated",
    "mirca_area_total",
    "mirca_rainfed",
    "yield",
    "production",
]
GLOBAL_WATER_MODELS = [
    "cwatm",
    "h08",
    "lpjml",
    "matsiro",
    "pcr-globwb",
    "watergap2",
]
CLIMATE_MODELS = ["gfdl-esm2m", "hadgem2-es", "ipsl-cm5a-lr", "miroc5"]
SCENARIOS = ["rcp26", "rcp60", "rcp85"]
VARIABLES = [
    "vwc",
    "vwcb",
    "vwcg",
    "vwcg_perc",
    "vwcb_perc",
    "wf",
    "wfb",
    "wfg",
    "etb",
    "etg",
    "rb",
    "rg",
    "wdb",
    "wdg",
]

# Set up logger with timestamp
logger = logging.getLogger(__name__)

app = typer.Typer(
    no_args_is_help=True,
    add_completion=False,
    pretty_exceptions_show_locals=False,
)


def traverse_directory_and_build_filenames(base_directory: str):
    """Traverse directories and build the filenames according to the folder structure."""
    files_to_upload = []

    for root, dirs, files in os.walk(base_directory):
        for file in files:
            if file.endswith(".tif"):
                file_lower = file.lower().replace(
                    ".tif", ""
                )  # Variable is the filename without extension
                relative_path = [
                    part.lower()
                    for part in os.path.relpath(root, base_directory).split(
                        os.sep
                    )
                ]

                # Check if it's a crop-specific directory
                if (
                    len(relative_path) >= 2
                    and relative_path[0] == "crop specific parameters"
                ):
                    crop_specific_dir = relative_path[
                        1
                    ]  # e.g., GeoTiff_Production, GeoTiff_MIRCA_Areas
                    crop_variable_mapping = {
                        "geotiff_production": "production",
                        "geotiff_mirca_areas": "area",
                        "geotiff_yield_gapfilled": "yield",
                    }

                    if crop_specific_dir.lower() in crop_variable_mapping:
                        crop = file_lower.split("_")[
                            0
                        ].lower()  # Extract crop from the file name
                        variable = crop_variable_mapping[
                            crop_specific_dir.lower()
                        ]  # Map folder name to variable

                        if (
                            crop in CROP_ITEMS
                            and variable in CROP_SPECIFIC_VARIABLES
                        ):
                            filename = f"{crop}_{variable}.tif"
                            file_path = os.path.join(root, file)
                            files_to_upload.append((file_path, filename))
                            logger.debug(
                                f"Valid crop-specific file: {filename} from {file_path}"
                            )
                        else:
                            logger.debug(
                                f"Skipping invalid crop-specific file: {file_lower} in {crop_specific_dir}"
                            )
                else:
                    # For general structure, check if "2005soc" or "historical" is part of the path and ignore them
                    path_filtered = [
                        part
                        for part in relative_path
                        if part not in ["2005soc", "historical"]
                    ]
                    if len(path_filtered) < 5:
                        logger.debug(
                            f"Skipping incomplete structure: {path_filtered}"
                        )
                        continue

                    water_model = path_filtered[0]
                    climate_model = path_filtered[1]
                    scenario = path_filtered[2]
                    year = path_filtered[3]
                    crop = path_filtered[4]
                    variable = file_lower

                    if all(
                        [
                            crop in CROP_ITEMS,
                            water_model in GLOBAL_WATER_MODELS,
                            climate_model in CLIMATE_MODELS,
                            scenario in SCENARIOS,
                            variable in VARIABLES,
                        ]
                    ):
                        filename = f"{crop}_{water_model}_{climate_model}_{scenario}_{variable}_{year}.tif"
                        file_path = os.path.join(root, file)
                        files_to_upload.append((file_path, filename))
                        logger.debug(
                            f"Valid general file: {filename} from {file_path}"
                        )
                    else:
                        logger.debug(
                            f"Skipping invalid file: {file_lower} in {root}"
                        )

    logger.info(
        f"Total valid files to upload (traverse mode): {len(files_to_upload)}"
    )
    return files_to_upload, len(files_to_upload)


def flattened_directory_build_filenames(flat_directory: str):
    """Process all .tif files in a flat directory."""
    files_to_upload = []

    for file in os.listdir(flat_directory):
        if file.endswith(".tif"):
            file_lower = file.lower().replace(
                ".tif", ""
            )  # Variable is the filename without extension
            file_parts = file_lower.split("_")

            if len(file_parts) == 6:  # General structure file
                crop, water_model, climate_model, scenario, variable, year = (
                    file_parts
                )
                if all(
                    [
                        crop in CROP_ITEMS,
                        water_model in GLOBAL_WATER_MODELS,
                        climate_model in CLIMATE_MODELS,
                        scenario in SCENARIOS,
                        variable in VARIABLES,
                    ]
                ):
                    files_to_upload.append(
                        (os.path.join(flat_directory, file), file)
                    )
            elif len(file_parts) == 2:  # Crop-specific structure
                crop, crop_specific_variable = file_parts
                if (
                    crop in CROP_ITEMS
                    and crop_specific_variable in CROP_SPECIFIC_VARIABLES
                ):
                    files_to_upload.append(
                        (os.path.join(flat_directory, file), file)
                    )
            else:
                logger.debug(f"Skipping invalid filename structure: {file}")

    logger.info(
        f"Total valid files to upload (flattened mode): {len(files_to_upload)}"
    )
    return files_to_upload, len(files_to_upload)


def load_token_cache():
    """Load the token cache from a file."""
    if os.path.exists(TOKEN_CACHE_FILE):
        with open(TOKEN_CACHE_FILE, "r") as file:
            return json.load(file)
    return None


def save_token_cache(token_data):
    """Save the token cache to a file."""
    with open(TOKEN_CACHE_FILE, "w") as file:
        json.dump(token_data, file)


def get_token_from_cache(keycloak_openid):
    """Retrieve and refresh the token from the cache if necessary."""
    token_data = load_token_cache()

    if token_data:
        try:
            new_token_data = keycloak_openid.refresh_token(
                token_data["refresh_token"]
            )
            save_token_cache(new_token_data)
            return new_token_data["access_token"]
        except Exception:
            logger.warning("Failed to refresh token, retrieving a new one.")
    return None


def get_new_token(keycloak_openid):
    """Get a new token using provided credentials."""
    username = input(f"Enter your {APPLICATION_NAME} username: ")
    password = getpass.getpass(f"Enter your {APPLICATION_NAME} password: ")

    token_data = keycloak_openid.token(username, password)
    save_token_cache(token_data)

    return token_data["access_token"]


def get_token(server: str):
    """Authenticate with Keycloak and obtain or refresh a token."""
    response = requests.get(f"{server}/api/config/keycloak")
    response.raise_for_status()
    keycloak_config = response.json()

    keycloak_openid = KeycloakOpenID(
        server_url=keycloak_config["url"],
        client_id=keycloak_config["clientId"],
        realm_name=keycloak_config["realm"],
        verify=True,
    )

    token = get_token_from_cache(keycloak_openid)
    logger.info("Token loaded from cache.")
    if not token:
        logger.info("No valid token found in cache.")
        token = get_new_token(keycloak_openid)
        logger.info("New token obtained.")

    return token


def upload_file(server, file_path, token, filename, overwrite_duplicates):
    """Upload a single file to the server."""
    with open(file_path, "rb") as f:
        files = {"file": (filename, f)}
        headers = {"Authorization": f"Bearer {token}"}

        response = requests.post(
            f"{server}{UPLOAD_ENDPOINT}",
            files=files,
            headers=headers,
            params={"overwrite_duplicates": overwrite_duplicates},
        )

    if response.status_code == 200:
        logger.info(f"Successfully uploaded {filename}")
    else:
        logger.error(
            f"Failed to upload {filename}: {response.status_code} - {response.text}"
        )


def parallel_upload(
    files_to_upload, server, token, threads, overwrite_duplicates
):
    """Upload files in parallel."""
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=threads
    ) as executor:
        futures = [
            executor.submit(
                upload_file,
                server,
                file_path,
                token,
                filename,
                overwrite_duplicates,
            )
            for file_path, filename in files_to_upload
        ]
        for future in concurrent.futures.as_completed(futures):
            future.result()


@app.command()
def traverse(
    directory: str,
    server: str = DROP4CROP_SERVER,
    threads: int = DEFAULT_THREADS,
    overwrite: bool = False,
    noconfirm: bool = False,
    debug: bool = False,
):
    """Traverse nested directories to find and upload files."""
    if debug:
        logging.basicConfig(
            level=logging.DEBUG,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )
    else:
        logging.basicConfig(
            level=logging.INFO,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )
    logging.info("Starting the uploader.")
    token = get_token(server)
    logging.info(f"Traversing folder: {directory}")
    files_to_upload, num_files = traverse_directory_and_build_filenames(
        directory
    )

    # Summary
    logger.info(f"Server: {server}")
    logger.info(f"Files to upload: {num_files}")
    logger.info(f"Threads: {threads}")
    logger.info(f"Overwrite duplicates: {'Yes' if overwrite else 'No'}")

    # Ask for confirmation unless --noconfirm is specified
    if not noconfirm:
        confirm = input("Do you want to proceed with these settings? [y/N]: ")
        if confirm.lower() != "y":
            print("Operation cancelled.")
            return

    # Perform the parallel upload
    parallel_upload(files_to_upload, server, token, threads, overwrite)


@app.command()
def flattened(
    directory: str,
    server: str = DROP4CROP_SERVER,
    threads: int = DEFAULT_THREADS,
    overwrite: bool = False,
    noconfirm: bool = False,
    debug: bool = False,
):
    """Upload files from a flat directory."""
    if debug:
        logging.basicConfig(
            level=logging.DEBUG,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )
    else:
        logging.basicConfig(
            level=logging.INFO,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )

    token = get_token(server)
    files_to_upload, num_files = flattened_directory_build_filenames(directory)

    # Summary
    logger.info(f"Server: {server}")
    logger.info(f"Files to upload: {num_files}")
    logger.info(f"Threads: {threads}")
    logger.info(f"Overwrite duplicates: {'Yes' if overwrite else 'No'}")

    # Ask for confirmation unless --noconfirm is specified
    if not noconfirm:
        confirm = input("Do you want to proceed with these settings? [y/N]: ")
        if confirm.lower() != "y":
            print("Operation cancelled.")
            return

    # Perform the parallel upload
    parallel_upload(files_to_upload, server, token, threads, overwrite)


if __name__ == "__main__":
    app()
